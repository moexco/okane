use crate::runtime::{EngineBase, PluginContext};
use okane_core::common::time::TimeProvider;
use futures::StreamExt;
use okane_core::common::TimeFrame;
use okane_core::engine::entity::Signal;
use okane_core::engine::error::EngineError;
use okane_core::market::port::Market;
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};
use wasmtime::*;

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
}

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
    ) -> Self {
        Self {
            base: EngineBase::new(market),
            trade_port,
            time_provider,
        }
    }

    /// # Summary
    /// 注册信号处理器。
    ///
    /// # Arguments
    /// * `handler`: 信号处理钩子实现。
    pub fn register_handler(&mut self, handler: Box<dyn okane_core::engine::port::SignalHandler>) {
        self.base.register_handler(handler);
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
    ///    b. 调用 on_candle(ptr, len) 并读取返回值。
    ///    c. 解析信号后通过 EngineBase 分发。
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
        }));

        let mut store = Store::new(&engine, plugin_ctx.clone());
        // 设置执行 fuel 限制（每次 on_candle 调用前重置）
        store
            .set_fuel(1_000_000)
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

        // 订阅 K 线流
        let mut stream = self.base.subscribe(symbol, timeframe).await?;

        // 核心执行循环
        while let Some(candle) = stream.next().await {
            // 注意: 在回测模式中 time_provider 应当由外部循环驱动，实盘中时间自动流逝

            // 重置 fuel
            store
                .set_fuel(1_000_000)
                .map_err(|e| EngineError::Plugin(e.to_string()))?;

            // 序列化 K 线数据
            let input_json = serde_json::to_string(&candle)
                .map_err(|e| EngineError::Plugin(e.to_string()))?;
            let input_bytes = input_json.as_bytes();

            // 在 WASM 中分配内存并写入输入数据
            let ptr = alloc_fn
                .call(&mut store, input_bytes.len() as i32)
                .map_err(|e| EngineError::Plugin(format!("alloc call failed: {}", e)))?;

            memory
                .write(&mut store, ptr as usize, input_bytes)
                .map_err(|e| EngineError::Plugin(format!("Memory write failed: {}", e)))?;

            // 调用 on_candle(ptr, len) -> result_ptr
            let result_ptr = on_candle
                .call(&mut store, (ptr, input_bytes.len() as i32))
                .map_err(|e| {
                    error!("WasmEngine: on_candle execution failed for {}: {}", symbol, e);
                    EngineError::Plugin(e.to_string())
                })?;

            // 读取返回结果
            if result_ptr != 0 {
                // 结果格式: 前 4 字节是长度，后面是 JSON 字符串
                let mut len_buf = [0u8; 4];
                memory
                    .read(&store, result_ptr as usize, &mut len_buf)
                    .map_err(|e| EngineError::Plugin(format!("Memory read failed: {}", e)))?;
                let result_len = u32::from_le_bytes(len_buf) as usize;

                let mut result_buf = vec![0u8; result_len];
                memory
                    .read(&store, (result_ptr as usize) + 4, &mut result_buf)
                    .map_err(|e| EngineError::Plugin(format!("Memory read failed: {}", e)))?;

                let result_str = String::from_utf8(result_buf)
                    .map_err(|e| EngineError::Plugin(format!("UTF-8 decode error: {}", e)))?;

                if let Ok(Some(signal)) = serde_json::from_str::<Option<Signal>>(&result_str) {
                    self.base.dispatch_signal(signal).await?;
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
                        let mut buf = vec![0u8; len as usize];
                        if memory.read(&caller, ptr as usize, &mut buf).is_ok() {
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
                |caller: Caller<'_, Arc<Mutex<PluginContext>>>| -> i64 {
                    let plugin_ctx = caller.data();
                    let ctx = plugin_ctx.lock().unwrap_or_else(|e| e.into_inner());
                    ctx.time_provider.now().timestamp_millis()
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
                 -> i32 {
                    let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                        Some(m) => m,
                        None => return 0,
                    };

                    // 读取 symbol 字符串
                    let mut sym_buf = vec![0u8; sym_len as usize];
                    if memory.read(&caller, sym_ptr as usize, &mut sym_buf).is_err() {
                        return 0;
                    }
                    let symbol = match String::from_utf8(sym_buf) {
                        Ok(s) => s,
                        Err(_) => return 0,
                    };

                    // 读取 timeframe 字符串
                    let mut tf_buf = vec![0u8; tf_len as usize];
                    if memory.read(&caller, tf_ptr as usize, &mut tf_buf).is_err() {
                        return 0;
                    }
                    let tf_str = match String::from_utf8(tf_buf) {
                        Ok(s) => s,
                        Err(_) => return 0,
                    };

                    let tf = match tf_str.parse::<TimeFrame>() {
                        Ok(t) => t,
                        Err(_) => return 0,
                    };

                    // 获取上下文数据
                    let plugin_ctx = caller.data().clone();
                    let (end_at, market) = {
                        let ctx = plugin_ctx.lock().unwrap_or_else(|e| e.into_inner());
                        (ctx.time_provider.now(), ctx.market.clone())
                    };

                    // 阻塞式桥接异步调用
                    let result = futures::executor::block_on(async {
                        let stock = market.get_stock(&symbol).await.map_err(|e| e.to_string())?;
                        // 计算起始时间 (limit * 周期 * 缓冲系数)
                        let duration = tf.duration() * (limit * 2);
                        let start = end_at - duration;
                        let end = end_at;

                        stock
                            .fetch_history(tf, start, end)
                            .await
                            .map(|h| h.into_iter().rev().take(limit as usize).rev().collect::<Vec<_>>())
                            .map_err(|e| e.to_string())
                    });

                    match result {
                        Ok(candles) => {
                            let json =
                                serde_json::to_string(&candles).unwrap_or_else(|_| "[]".to_string());
                            let json_bytes = json.as_bytes();
                            // 写入长度（4字节 LE）+ JSON 数据
                            let len_bytes = (json_bytes.len() as u32).to_le_bytes();
                            if memory
                                .write(&mut caller, out_ptr as usize, &len_bytes)
                                .is_err()
                            {
                                return 0;
                            }
                            if memory
                                .write(
                                    &mut caller,
                                    (out_ptr as usize) + 4,
                                    json_bytes,
                                )
                                .is_err()
                            {
                                return 0;
                            }
                            json_bytes.len() as i32
                        }
                        Err(_) => 0,
                    }
                },
            )
            .map_err(|e| EngineError::Plugin(e.to_string()))?;

        // 辅助闭包：处理报单逻辑（买/卖）
        let process_order = |
            mut caller: Caller<'_, Arc<Mutex<PluginContext>>>,
            sym_ptr: i32,
            sym_len: i32,
            price_f64: f64, // 使用 f64 传递价格以简化 FFI
            vol_f64: f64,   // 使用 f64 传递数量以简化 FFI
            out_ptr: i32,
            direction: okane_core::trade::entity::OrderDirection,
        | -> i32 {
            let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
                Some(m) => m,
                None => return 0,
            };

            // 读取 symbol
            let mut sym_buf = vec![0u8; sym_len as usize];
            if memory.read(&caller, sym_ptr as usize, &mut sym_buf).is_err() {
                return 0;
            }
            let symbol = match String::from_utf8(sym_buf) {
                Ok(s) => s,
                Err(_) => return 0,
            };

            // 获取 trade_port 和 account_id
            let plugin_ctx = caller.data().clone();
            let (trade_port, account_id) = {
                let ctx = plugin_ctx.lock().unwrap_or_else(|e| e.into_inner());
                (ctx.trade_port.clone(), ctx.account_id.clone())
            };

            let req_price = if price_f64 > 0.0 {
                rust_decimal::Decimal::from_f64_retain(price_f64)
            } else {
                None
            };
            let req_vol = rust_decimal::Decimal::from_f64_retain(vol_f64).unwrap_or(rust_decimal::Decimal::ZERO);

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
            let result = futures::executor::block_on(async {
                trade_port.submit_order(order).await
            });

            match result {
                Ok(oid) => {
                    let json = format!("{{\"order_id\": \"{}\"}}", oid.0);
                    let json_bytes = json.as_bytes();
                    let len_bytes = (json_bytes.len() as u32).to_le_bytes();
                    if memory.write(&mut caller, out_ptr as usize, &len_bytes).is_err() { return 0; }
                    if memory.write(&mut caller, (out_ptr as usize) + 4, json_bytes).is_err() { return 0; }
                    json_bytes.len() as i32
                }
                Err(e) => {
                    let json = format!("{{\"error\": \"{}\"}}", e);
                    let json_bytes = json.as_bytes();
                    let len_bytes = (json_bytes.len() as u32).to_le_bytes();
                    if memory.write(&mut caller, out_ptr as usize, &len_bytes).is_err() { return 0; }
                    if memory.write(&mut caller, (out_ptr as usize) + 4, json_bytes).is_err() { return 0; }
                    json_bytes.len() as i32
                }
            }
        };

        // host_buy(sym_ptr: i32, sym_len: i32, price: f64, vol: f64, out_ptr: i32) -> i32
        linker
            .func_wrap("env", "host_buy", move |caller: Caller<'_, Arc<Mutex<PluginContext>>>, sym_ptr: i32, sym_len: i32, price: f64, vol: f64, out_ptr: i32| -> i32 {
                process_order(caller, sym_ptr, sym_len, price, vol, out_ptr, okane_core::trade::entity::OrderDirection::Buy)
            })
            .map_err(|e| EngineError::Plugin(e.to_string()))?;

        // host_sell(sym_ptr: i32, sym_len: i32, price: f64, vol: f64, out_ptr: i32) -> i32
        linker
            .func_wrap("env", "host_sell", move |caller: Caller<'_, Arc<Mutex<PluginContext>>>, sym_ptr: i32, sym_len: i32, price: f64, vol: f64, out_ptr: i32| -> i32 {
                process_order(caller, sym_ptr, sym_len, price, vol, out_ptr, okane_core::trade::entity::OrderDirection::Sell)
            })
            .map_err(|e| EngineError::Plugin(e.to_string()))?;

        Ok(())
    }
}
