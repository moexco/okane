use crate::runtime::{EngineBase, PluginContext};
use okane_core::common::time::TimeProvider;
use futures::StreamExt;
use okane_core::common::TimeFrame;
use okane_core::engine::error::EngineError;
use okane_core::market::port::Market;
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};
use wasmtime::*;
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::trade::entity::OrderId;

/// # Summary
/// 基于 wasmtime 的策略执行引擎。
///
/// # Invariants
/// - 执行由 Rust/C 等编译语言编译而成的 WASM 模块。
/// - WASM 沙盒内仅能访问宿主通过 Linker 注册的 import 函数。
/// - 适用于生产运行或第三方高性能策略插件。
pub struct WasmEngine {
    pub base: EngineBase,
    trade_port: Arc<dyn okane_core::trade::port::TradePort>,
    time_provider: Arc<dyn TimeProvider>,
    notifier: Option<Arc<dyn okane_core::notify::port::Notifier>>,
    bridge: Arc<crate::bridge::AsyncBridge>,
}

const WASM_FUEL_LIMIT: u64 = 1_000_000;

impl WasmEngine {
    /// # Summary
    /// 创建 WasmEngine 实例。
    ///
    /// # Arguments
    /// * `market`: 市场数据驱动接口。
    ///
    /// # Returns
    /// * `Self` - 初始化后的引擎实例。
    pub fn new(
        market: Arc<dyn Market>,
        trade_port: Arc<dyn okane_core::trade::port::TradePort>,
        time_provider: Arc<dyn TimeProvider>,
        notifier: Option<Arc<dyn okane_core::notify::port::Notifier>>,
    ) -> Result<Self, EngineError> {
        Ok(Self {
            base: EngineBase::new(market),
            trade_port,
            time_provider,
            notifier,
            bridge: Arc::new(crate::bridge::AsyncBridge::new()?),
        })
    }

    /// # Summary
    /// 运行 WASM 策略。
    ///
    /// # Logic
    /// 1. 初始化 wasmtime Engine 和 Store，配置 fuel 限制。
    /// 2. 通过 Linker 注册宿主函数（host_log, host_now, host_fetch_history）。
    /// 3. 编译并实例化 WASM 模块。
    /// 4. 获取 WASM 导出的内存和 on_candle 函数。
    /// 5. 订阅 K 线流，对每根 K 线：
    ///    a. 将 JSON 写入 WASM 线性内存。
    ///    b. 调用 on_candle(ptr, len)，策略通过 host.* API 直接执行动作。
    ///
    /// # Arguments
    /// * `symbol`: 证券代码。
    /// * `timeframe`: K 线时间周期。
    /// * `wasm_bytes`: 策略 WASM 模块的字节码。
    ///
    /// # Returns
    /// * `Result<(), EngineError>` - 成功或错误。
    pub async fn run_strategy(
        &self,
        symbol: &str,
        account_id: &str,
        timeframe: TimeFrame,
        wasm_bytes: &[u8],
    ) -> Result<(), EngineError> {
        info!(
            "WasmEngine: Starting strategy for {} with timeframe {:?}",
            symbol, timeframe
        );

        // 配置 wasmtime
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine =
            Engine::new(&config).map_err(|e| EngineError::Plugin(e.to_string()))?;

        let plugin_ctx = Arc::new(Mutex::new(PluginContext {
            market: self.base.market.clone(),
            trade_port: self.trade_port.clone(),
            account_id: account_id.to_string(),
            time_provider: self.time_provider.clone(),
            notifier: self.notifier.clone(),
            bridge: self.bridge.clone(),
        }));

        let mut store = Store::new(&engine, plugin_ctx.clone());
        // 设置执行 fuel 限制（每次 on_candle 调用前重置）
        store
            .set_fuel(WASM_FUEL_LIMIT)
            .map_err(|e| EngineError::Plugin(e.to_string()))?;

        // 编译 WASM 模块
        let module = Module::new(&engine, wasm_bytes)
            .map_err(|e| EngineError::Plugin(format!("WASM compilation error: {}", e)))?;

        // 通过 Linker 注册宿主函数
        let mut linker: Linker<Arc<Mutex<PluginContext>>> = Linker::new(&engine);
        Self::register_host_functions(&mut linker)?;

        // 实例化模块
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| EngineError::Plugin(format!("WASM instantiation error: {}", e)))?;

        // 获取导出的函数和内存
        let on_candle = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "on_candle")
            .map_err(|e| {
                EngineError::Plugin(format!("on_candle export not found: {}", e))
            })?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| EngineError::Plugin("memory export not found".to_string()))?;

        // 获取 WASM alloc/dealloc 函数（策略需导出这些函数用于内存管理）
        let alloc_fn = instance
            .get_typed_func::<i32, i32>(&mut store, "alloc")
            .map_err(|e| {
                EngineError::Plugin(format!("alloc export not found: {}", e))
            })?;

        let dealloc_fn = instance
            .get_typed_func::<(i32, i32), ()>(&mut store, "dealloc")
            .map_err(|e| {
                EngineError::Plugin(format!("dealloc export not found: {}", e))
            })?;

        // 订阅 K 线流
        let mut stream = self.base.subscribe(symbol, timeframe).await?;

        // 核心执行循环
        while let Some(candle) = stream.next().await {

            // 重置 fuel
            store
                .set_fuel(WASM_FUEL_LIMIT)
                .map_err(|e| EngineError::Plugin(e.to_string()))?;

            // 序列化 K 线数据
            let input_json = serde_json::to_string(&candle)
                .map_err(|e| EngineError::Plugin(e.to_string()))?;
            let input_bytes = input_json.as_bytes();

            // 在 WASM 中分配内存并写入输入数据
            let input_len_i32 = i32::try_from(input_bytes.len()).map_err(|e| EngineError::Plugin(format!("Input size overflow: {}", e)))?;
            let ptr = alloc_fn
                .call(&mut store, input_len_i32)
                .map_err(|e| EngineError::Plugin(format!("alloc call failed: {}", e)))?;

            let usize_ptr = usize::try_from(ptr).map_err(|e| EngineError::Plugin(format!("Invalid pointer from WASM: {}", e)))?;
            memory
                .write(&mut store, usize_ptr, input_bytes)
                .map_err(|e| EngineError::Plugin(format!("Memory write failed: {}", e)))?;

            // 调用 on_candle(ptr, len) -> result_ptr
            let result_ptr = match on_candle.call(&mut store, (ptr, input_len_i32)) {
                Ok(res) => res,
                Err(e) => {
                    // 若执行抛错，也需尝试释放入参指针。如果释放也失败，记录警告但返回原始执行错误。
                    if let Err(dealloc_e) = dealloc_fn.call(&mut store, (ptr, input_len_i32)) {
                        error!("WasmEngine: fatal dealloc failure after execution error: {}", dealloc_e);
                    }
                    error!("WasmEngine: on_candle execution failed for {}: {}", symbol, e);
                    return Err(EngineError::Plugin(e.to_string()));
                }
            };

            // 无论正常与否，由宿主分配的入参必须在通信完成后由宿主要求释放
            dealloc_fn.call(&mut store, (ptr, input_len_i32))
                .map_err(|e| EngineError::Plugin(format!("Failed to dealloc input buffer: {}", e)))?;

            if result_ptr != 0 {
                let mut len_bytes = [0u8; 4];
                let usize_result_ptr = usize::try_from(result_ptr).map_err(|e| EngineError::Plugin(format!("Invalid result pointer: {}", e)))?;
                if memory.read(&store, usize_result_ptr, &mut len_bytes).is_ok() {
                    let json_len = i32::try_from(u32::from_le_bytes(len_bytes)).map_err(|e| EngineError::Plugin(format!("Invalid result length: {}", e)))?;
                    let total_len = json_len + 4;
                    // 彻底释放策略侧通过 alloc 构建的返回值内存
                    dealloc_fn.call(&mut store, (result_ptr, total_len))
                        .map_err(|e| EngineError::Plugin(format!("Failed to dealloc result buffer: {}", e)))?;
                }
            }
        }

        Ok(())
    }

    /// # Summary
    /// 在 Linker 中注册宿主函数。
    ///
    /// # Logic
    /// 注册以下 import 函数到 "env" 模块命名空间：
    /// - `host_log(level: i32, ptr: i32, len: i32)` — 日志输出
    /// - `host_now() -> i64` — 逻辑时间戳
    /// - `host_fetch_history(sym_ptr, sym_len, tf_ptr, tf_len, limit, out_ptr) -> i32` — 历史 K 线
    fn register_host_functions(
        linker: &mut Linker<Arc<Mutex<PluginContext>>>,
    ) -> Result<(), EngineError> {
        // host_log(level: i32, ptr: i32, len: i32)
        linker
            .func_wrap(
                "env",
                "host_log",
                |mut caller: Caller<'_, Arc<Mutex<PluginContext>>>,
                 level: i32,
                 ptr: i32,
                 len: i32| {
                    let memory = caller.get_export("memory").and_then(|e| e.into_memory());
                    if let Some(memory) = memory {
                        let usize_len = match usize::try_from(len) {
                            Ok(l) => l,
                            Err(_) => return,
                        };
                        let mut buf = vec![0u8; usize_len];
                        let usize_ptr = match usize::try_from(ptr) {
                            Ok(p) => p,
                            Err(_) => return,
                        };
                        if memory.read(&caller, usize_ptr, &mut buf).is_ok() {
                            let msg = String::from_utf8_lossy(&buf);
                            match level {
                                1 => error!("WASM [ERROR]: {}", msg),
                                2 => warn!("WASM [WARN]: {}", msg),
                                _ => info!("WASM [INFO]: {}", msg),
                            }
                        }
                    }
                },
            )
            .map_err(|e| EngineError::Plugin(e.to_string()))?;

        // host_now() -> i64
        linker
            .func_wrap(
                "env",
                "host_now",
                |caller: Caller<'_, Arc<Mutex<PluginContext>>>| -> Result<i64, anyhow::Error> {
                    let plugin_ctx = caller.data();
                    let ctx = plugin_ctx.lock().map_err(|e| anyhow::anyhow!("Plugin context poisoned: {}", e))?;
                    ctx.time_provider.now()
                        .map(|t| t.timestamp_millis())
                        .map_err(|e| anyhow::anyhow!(e.to_string()))
                },
            )
            .map_err(|e| EngineError::Plugin(e.to_string()))?;

        // host_fetch_history(sym_ptr, sym_len, tf_ptr, tf_len, limit, out_ptr) -> i32
        // 返回值: 写入 out_ptr 的字节数，0 表示错误
        linker
            .func_wrap(
                "env",
                "host_fetch_history",
                |mut caller: Caller<'_, Arc<Mutex<PluginContext>>>,
                 sym_ptr: i32,
                 sym_len: i32,
                 tf_ptr: i32,
                 tf_len: i32,
                 limit: i32,
                 out_ptr: i32|
                 -> Result<i32, anyhow::Error> {
                    let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                        Some(m) => m,
                        None => return Ok(0),
                    };

                    // 读取 symbol 字符串
                    let usize_sym_len = usize::try_from(sym_len).map_err(|e| anyhow::anyhow!("Invalid sym_len: {}", e))?;
                    let mut sym_buf = vec![0u8; usize_sym_len];
                    let usize_sym_ptr = usize::try_from(sym_ptr).map_err(|e| anyhow::anyhow!("Invalid sym_ptr: {}", e))?;
                    if memory.read(&caller, usize_sym_ptr, &mut sym_buf).is_err() {
                        return Ok(0);
                    }
                    let symbol = match String::from_utf8(sym_buf) {
                        Ok(s) => s,
                        Err(_) => return Ok(0),
                    };

                    // 读取 timeframe 字符串
                    let usize_tf_len = usize::try_from(tf_len).map_err(|e| anyhow::anyhow!("Invalid tf_len: {}", e))?;
                    let mut tf_buf = vec![0u8; usize_tf_len];
                    let usize_tf_ptr = usize::try_from(tf_ptr).map_err(|e| anyhow::anyhow!("Invalid tf_ptr: {}", e))?;
                    if memory.read(&caller, usize_tf_ptr, &mut tf_buf).is_err() {
                        return Ok(0);
                    }
                    let tf_str = match String::from_utf8(tf_buf) {
                        Ok(s) => s,
                        Err(_) => return Ok(0),
                    };

                    let tf_parsed = match tf_str.parse::<TimeFrame>() {
                        Ok(t) => t,
                        Err(_) => return Ok(0),
                    };

                    // 获取上下文数据
                    let plugin_ctx = caller.data().clone();
                    let (end_at, market, bridge) = {
                        let ctx = plugin_ctx.lock().map_err(|e| anyhow::anyhow!("Plugin context poisoned: {}", e))?;
                        (ctx.time_provider.now().map_err(|e| anyhow::anyhow!(e.to_string()))?, ctx.market.clone(), ctx.bridge.clone())
                    };

                    // 阻塞式桥接异步调用
                    let result: Result<Result<Vec<Candle>, String>, anyhow::Error> = bridge.call(async move {
                        let stock = market.get_stock(&symbol).await.map_err(|e: MarketError| e.to_string())?;
                        // 计算起始时间 (limit * 周期 * 缓冲系数)
                        let duration = tf_parsed.duration() * (limit * 2);
                        let start = end_at - duration;
                        let end = end_at;

                        let h = stock
                            .fetch_history(tf_parsed, start, end)
                            .await
                            .map_err(|e: MarketError| e.to_string())?;
                        
                        let usize_limit = usize::try_from(limit).map_err(|e| format!("Invalid limit: {}", e))?;
                        Ok(h.into_iter().rev().take(usize_limit).rev().collect::<Vec<_>>())
                    }).map_err(|e: EngineError| anyhow::anyhow!(e.to_string()));

                    match result {
                        Ok(Ok(candles)) => {
                            let json = serde_json::to_string(&candles).map_err(|e| anyhow::anyhow!(e.to_string()))?;
                            let json_bytes = json.as_bytes();
                            // 写入长度（4字节 LE）+ JSON 数据
                            let json_len_u32 = u32::try_from(json_bytes.len()).map_err(|e| anyhow::anyhow!(e.to_string()))?;
                            let len_bytes = json_len_u32.to_le_bytes();
                            let usize_out_ptr = usize::try_from(out_ptr).map_err(|e| anyhow::anyhow!(e.to_string()))?;
                            if memory.write(&mut caller, usize_out_ptr, &len_bytes).is_err() { return Ok(0); }
                            if memory.write(&mut caller, usize_out_ptr + 4, json_bytes).is_err() { return Ok(0); }
                            Ok(i32::try_from(json_bytes.len()).map_err(|e| anyhow::anyhow!("JSON len overflow: {}", e))?)
                        }
                        _ => Ok(0),
                    }
                },
            )
            .map_err(|e| EngineError::Plugin(e.to_string()))?;

        // 辅助闭包：处理报单逻辑（买/卖）
        let process_order = |
            mut caller: Caller<'_, Arc<Mutex<PluginContext>>>,
            sym_ptr: i32,
            sym_len: i32,
            price_ptr: i32, // 改为字符串指针以维持精度
            price_len: i32,
            vol_ptr: i32,   // 改为字符串指针
            vol_len: i32,
            out_ptr: i32,
            direction: okane_core::trade::entity::OrderDirection,
        | -> Result<i32, anyhow::Error> {
            let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m,
                None => return Ok(0),
            };

            // 1. 读取 symbol
            let usize_sym_len = usize::try_from(sym_len).map_err(|e| anyhow::anyhow!("Invalid sym_len: {}", e))?;
            let mut sym_buf = vec![0u8; usize_sym_len];
            let usize_sym_ptr = usize::try_from(sym_ptr).map_err(|e| anyhow::anyhow!("Invalid sym_ptr: {}", e))?;
            if memory.read(&caller, usize_sym_ptr, &mut sym_buf).is_err() { return Ok(0); }
            let symbol = String::from_utf8_lossy(&sym_buf).to_string();

            // 2. 读取 price (String)
            let req_price = if price_ptr != 0 {
                let usize_price_len = usize::try_from(price_len).map_err(|e| anyhow::anyhow!("Invalid price_len: {}", e))?;
                let mut price_buf = vec![0u8; usize_price_len];
                let usize_price_ptr = usize::try_from(price_ptr).map_err(|e| anyhow::anyhow!("Invalid price_ptr: {}", e))?;
                if memory.read(&caller, usize_price_ptr, &mut price_buf).is_err() { return Ok(0); }
                let price_str = String::from_utf8_lossy(&price_buf);
                let dec = price_str.parse::<rust_decimal::Decimal>().map_err(|e| anyhow::anyhow!("Invalid price string: {}", e))?;
                Some(dec)
            } else {
                None
            };

            // 3. 读取 volume (String)
            let usize_vol_len = usize::try_from(vol_len).map_err(|e| anyhow::anyhow!("Invalid vol_len: {}", e))?;
            let mut vol_buf = vec![0u8; usize_vol_len];
            let usize_vol_ptr = usize::try_from(vol_ptr).map_err(|e| anyhow::anyhow!("Invalid vol_ptr: {}", e))?;
            if memory.read(&caller, usize_vol_ptr, &mut vol_buf).is_err() { return Ok(0); }
            let vol_str = String::from_utf8_lossy(&vol_buf);
            let req_vol = vol_str.parse::<rust_decimal::Decimal>().map_err(|e| anyhow::anyhow!("Invalid volume string: {}", e))?;

            // 获取 trade_port 和 account_id 和 bridge
            let plugin_ctx = caller.data().clone();
            let (trade_port, account_id, bridge) = {
                let ctx = plugin_ctx.lock().map_err(|e| anyhow::anyhow!("Plugin context poisoned: {}", e))?;
                (ctx.trade_port.clone(), ctx.account_id.clone(), ctx.bridge.clone())
            };

            let order = okane_core::trade::entity::Order::new(
                okane_core::trade::entity::OrderId(uuid::Uuid::new_v4().to_string()),
                okane_core::trade::entity::AccountId(account_id),
                symbol,
                direction,
                req_price,
                req_vol,
                0, // mock strategy id
            );

            // 阻塞调用下单
            let result: Result<Result<OrderId, String>, anyhow::Error> = bridge.call(async move {
                trade_port.submit_order(order).await.map_err(|e| e.to_string())
            }).map_err(|e| anyhow::anyhow!(e.to_string()));

            match result {
                Ok(Ok(oid)) => {
                    let json = serde_json::json!({"order_id": oid.0}).to_string();
                    let json_bytes = json.as_bytes();
                    let json_len_u32 = u32::try_from(json_bytes.len()).map_err(|e| anyhow::anyhow!(e))?;
                    let len_bytes = json_len_u32.to_le_bytes();
                    let usize_out_ptr = usize::try_from(out_ptr).map_err(|e| anyhow::anyhow!(e))?;
                    if memory.write(&mut caller, usize_out_ptr, &len_bytes).is_err() { return Ok(0); }
                    if memory.write(&mut caller, usize_out_ptr + 4, json_bytes).is_err() { return Ok(0); }
                    Ok(i32::try_from(json_bytes.len()).map_err(|e| anyhow::anyhow!(e))?)
                }
                _ => {
                    let err_msg = match result {
                        Ok(Err(e)) => e,
                        Err(e) => e.to_string(),
                        _ => "Unknown error".to_string(),
                    };
                    let json = serde_json::json!({"error": err_msg}).to_string();
                    let json_bytes = json.as_bytes();
                    let json_len_u32 = u32::try_from(json_bytes.len()).map_err(|e| anyhow::anyhow!(e))?;
                    let len_bytes = json_len_u32.to_le_bytes();
                    let usize_out_ptr = usize::try_from(out_ptr).map_err(|e| anyhow::anyhow!(e))?;
                    memory.write(&mut caller, usize_out_ptr, &len_bytes).ok();
                    memory.write(&mut caller, usize_out_ptr + 4, json_bytes).ok();
                    Ok(i32::try_from(json_bytes.len()).map_err(|e| anyhow::anyhow!(e))?)
                }
            }
        };

        // host_buy(sym_ptr, sym_len, price_ptr, price_len, vol_ptr, vol_len, out_ptr)
        linker
            .func_wrap("env", "host_buy", move |caller: Caller<'_, Arc<Mutex<PluginContext>>>, sym_ptr: i32, sym_len: i32, price_ptr: i32, price_len: i32, vol_ptr: i32, vol_len: i32, out_ptr: i32| -> Result<i32, anyhow::Error> {
                process_order(caller, sym_ptr, sym_len, price_ptr, price_len, vol_ptr, vol_len, out_ptr, okane_core::trade::entity::OrderDirection::Buy)
            })
            .map_err(|e| EngineError::Plugin(e.to_string()))?;

        // host_sell(sym_ptr, sym_len, price_ptr, price_len, vol_ptr, vol_len, out_ptr)
        linker
            .func_wrap("env", "host_sell", move |caller: Caller<'_, Arc<Mutex<PluginContext>>>, sym_ptr: i32, sym_len: i32, price_ptr: i32, price_len: i32, vol_ptr: i32, vol_len: i32, out_ptr: i32| -> Result<i32, anyhow::Error> {
                process_order(caller, sym_ptr, sym_len, price_ptr, price_len, vol_ptr, vol_len, out_ptr, okane_core::trade::entity::OrderDirection::Sell)
            })
            .map_err(|e| EngineError::Plugin(e.to_string()))?;

        Ok(())
    }
}
