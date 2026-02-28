use crate::runtime::{EngineBase, PluginContext};
use futures::StreamExt;
use okane_core::common::TimeFrame;
use okane_core::common::time::TimeProvider;
use okane_core::engine::entity::Signal;
use okane_core::engine::error::EngineError;
use okane_core::market::port::Market;
use rquickjs::{AsyncContext, AsyncRuntime, Function, Object, Value, async_with};
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

/// # Summary
/// 基于 QuickJS 的策略执行引擎。
///
/// # Invariants
/// - 执行 JS 策略源码，无需编译步骤。
/// - JS 沙盒内无任何 I/O 能力，仅可调用宿主注入的 `host` 对象方法。
/// - 适用于策略开发、调试和回测场景。
pub struct JsEngine {
    pub base: EngineBase,
    trade_port: Arc<dyn okane_core::trade::port::TradePort>,
    time_provider: Arc<dyn TimeProvider>,
}

impl JsEngine {
    /// # Summary
    /// 创建 JsEngine 实例。
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
    /// 运行 JS 策略。
    ///
    /// # Logic
    /// 1. 创建 QuickJS AsyncRuntime 和 AsyncContext，配置内存与栈大小限制。
    /// 2. 在 JS 全局注入 `host` 对象，包含 log/now/fetchHistory 方法。
    /// 3. 加载并执行策略 JS 源码。
    /// 4. 订阅 K 线流，每次到达时序列化为 JSON 调用 JS 的 `onCandle` 函数。
    /// 5. 解析返回值为 `Option<Signal>`，若有信号则通过 EngineBase 分发。
    ///
    /// # Arguments
    /// * `symbol`: 证券代码。
    /// * `timeframe`: K 线时间周期。
    /// * `js_source`: 策略 JS 源码。
    ///
    /// # Returns
    /// * `Result<(), EngineError>` - 成功或错误。
    pub async fn run_strategy(
        &self,
        symbol: &str,
        account_id: &str,
        timeframe: TimeFrame,
        js_source: &str,
    ) -> Result<(), EngineError> {
        info!(
            "JsEngine: Starting strategy for {} with timeframe {:?}",
            symbol, timeframe
        );

        let rt = AsyncRuntime::new().map_err(|e| EngineError::Plugin(e.to_string()))?;

        // 设置内存限制：32MB
        rt.set_memory_limit(32 * 1024 * 1024).await;

        // 设置最大栈大小：1MB
        rt.set_max_stack_size(1024 * 1024).await;

        let ctx = AsyncContext::full(&rt)
            .await
            .map_err(|e| EngineError::Plugin(e.to_string()))?;

        // 共享的插件上下文
        let plugin_ctx = Arc::new(Mutex::new(PluginContext {
            market: self.base.market.clone(),
            trade_port: self.trade_port.clone(),
            account_id: account_id.to_string(),
            time_provider: self.time_provider.clone(),
        }));

        // 在 JS 全局注入 host 对象并加载策略源码
        let js_source_owned = js_source.to_string();
        let plugin_ctx_clone = plugin_ctx.clone();

        async_with!(ctx => |ctx| {
            if let Err(e) = Self::setup_host_and_load(&ctx, &js_source_owned, plugin_ctx_clone) {
                error!("JsEngine: Failed to setup host or load strategy: {}", e);
            }
        })
        .await;

        // 订阅 K 线流
        let mut stream = self.base.subscribe(symbol, timeframe).await?;

        // 核心执行循环
        while let Some(candle) = stream.next().await {
            // 注意: 在回测模式中 time_provider 应当由外部循环驱动，实盘中时间自动流逝

            // 序列化 K 线数据
            let candle_json = serde_json::to_string(&candle)
                .map_err(|e| EngineError::Plugin(e.to_string()))?;

            // 调用 JS 的 onCandle 函数
            let candle_json_clone = candle_json.clone();
            let result: Result<Option<Signal>, EngineError> = async_with!(ctx => |ctx| {
                Self::call_on_candle(&ctx, &candle_json_clone)
            })
            .await;

            match result {
                Ok(Some(signal)) => {
                    self.base.dispatch_signal(signal).await?;
                }
                Ok(None) => {}
                Err(e) => {
                    error!("JsEngine: Strategy execution failed for {}: {}", symbol, e);
                    return Err(e);
                }
            }
        }

        Ok(())
    }

    /// # Summary
    /// 在 JS 上下文中注入 host 对象并加载策略源码。
    ///
    /// # Logic
    /// 1. 创建 `host` JS 对象。
    /// 2. 注册 `host.log(level, msg)` — 调用宿主 tracing 系统。
    /// 3. 注册 `host.now()` — 返回当前逻辑时间戳（毫秒）。
    /// 4. 注册 `host.fetchHistory(symbol, tf, limit)` — 拉取历史 K 线（阻塞式桥接）。
    /// 5. 评估策略 JS 源码。
    fn setup_host_and_load(
        ctx: &rquickjs::Ctx<'_>,
        js_source: &str,
        plugin_ctx: Arc<Mutex<PluginContext>>,
    ) -> Result<(), EngineError> {
        let globals = ctx.globals();
        let host = Object::new(ctx.clone()).map_err(|e| EngineError::Plugin(e.to_string()))?;

        // host.log(level: number, msg: string)
        host.set(
            "log",
            Function::new(ctx.clone(), |level: i32, msg: String| {
                match level {
                    1 => error!("JS [ERROR]: {}", msg),
                    2 => warn!("JS [WARN]: {}", msg),
                    _ => info!("JS [INFO]: {}", msg),
                }
            })
            .map_err(|e| EngineError::Plugin(e.to_string()))?,
        )
        .map_err(|e| EngineError::Plugin(e.to_string()))?;

        // host.now() -> number (milliseconds)
        let ctx_for_now = plugin_ctx.clone();
        host.set(
            "now",
            Function::new(ctx.clone(), move || -> i64 {
                let ctx = ctx_for_now.lock().unwrap_or_else(|e| e.into_inner());
                ctx.time_provider.now().timestamp_millis()
            })
            .map_err(|e| EngineError::Plugin(e.to_string()))?,
        )
        .map_err(|e| EngineError::Plugin(e.to_string()))?;

        // host.fetchHistory(symbol: string, tf: string, limit: number) -> string (JSON)
        let ctx_for_fetch = plugin_ctx.clone();
        host.set(
            "fetchHistory",
            Function::new(ctx.clone(), move |symbol: String, tf: String, limit: i32| -> String {
                let ctx_mutex = ctx_for_fetch.lock().unwrap_or_else(|e| e.into_inner());
                let end_at = Some(ctx_mutex.time_provider.now());
                let market = ctx_mutex.market.clone();
                drop(ctx_mutex);

                let tf_parsed = match tf.parse::<TimeFrame>() {
                    Ok(t) => t,
                    Err(e) => return format!("{{\"error\": \"{}\"}}", e),
                };

                // 阻塞式桥接异步调用
                match futures::executor::block_on(async {
                    let stock = market.get_stock(&symbol).await.map_err(|e| e.to_string())?;
                    stock
                        .fetch_history(tf_parsed, limit as usize, end_at)
                        .await
                        .map_err(|e| e.to_string())
                }) {
                    Ok(candles) => serde_json::to_string(&candles).unwrap_or_else(|e| {
                        format!("{{\"error\": \"{}\"}}", e)
                    }),
                    Err(e) => format!("{{\"error\": \"{}\"}}", e),
                }
            })
            .map_err(|e| EngineError::Plugin(e.to_string()))?,
        )
        .map_err(|e| EngineError::Plugin(e.to_string()))?;

        // host.buy(symbol: string, price: number | null, volume: number) -> string (OrderId | Error)
        let ctx_for_buy = plugin_ctx.clone();
        host.set(
            "buy",
            Function::new(ctx.clone(), move |symbol: String, price: Option<f64>, volume: f64| -> String {
                let ctx_mutex = ctx_for_buy.lock().unwrap_or_else(|e| e.into_inner());
                let trade_port = ctx_mutex.trade_port.clone();
                let account_id = ctx_mutex.account_id.clone();
                drop(ctx_mutex);

                let req_price = price.and_then(rust_decimal::Decimal::from_f64_retain);
                let req_vol = rust_decimal::Decimal::from_f64_retain(volume).unwrap_or(rust_decimal::Decimal::ZERO);
                
                let order = okane_core::trade::entity::Order::new(
                    okane_core::trade::entity::OrderId(uuid::Uuid::new_v4().to_string()),
                    okane_core::trade::entity::AccountId(account_id),
                    symbol,
                    okane_core::trade::entity::OrderDirection::Buy,
                    req_price,
                    req_vol,
                    0,
                );

                match futures::executor::block_on(async {
                    trade_port.submit_order(order).await
                }) {
                    Ok(oid) => oid.0,
                    Err(e) => format!("{{\"error\": \"{}\"}}", e),
                }
            })
            .map_err(|e| EngineError::Plugin(e.to_string()))?,
        )
        .map_err(|e| EngineError::Plugin(e.to_string()))?;

        // host.sell(symbol: string, price: number | null, volume: number) -> string (OrderId | Error)
        let ctx_for_sell = plugin_ctx.clone();
        host.set(
            "sell",
            Function::new(ctx.clone(), move |symbol: String, price: Option<f64>, volume: f64| -> String {
                let ctx_mutex = ctx_for_sell.lock().unwrap_or_else(|e| e.into_inner());
                let trade_port = ctx_mutex.trade_port.clone();
                let account_id = ctx_mutex.account_id.clone();
                drop(ctx_mutex);

                let req_price = price.and_then(rust_decimal::Decimal::from_f64_retain);
                let req_vol = rust_decimal::Decimal::from_f64_retain(volume).unwrap_or(rust_decimal::Decimal::ZERO);
                
                let order = okane_core::trade::entity::Order::new(
                    okane_core::trade::entity::OrderId(uuid::Uuid::new_v4().to_string()),
                    okane_core::trade::entity::AccountId(account_id),
                    symbol,
                    okane_core::trade::entity::OrderDirection::Sell,
                    req_price,
                    req_vol,
                    0,
                );

                match futures::executor::block_on(async {
                    trade_port.submit_order(order).await
                }) {
                    Ok(oid) => oid.0,
                    Err(e) => format!("{{\"error\": \"{}\"}}", e),
                }
            })
            .map_err(|e| EngineError::Plugin(e.to_string()))?,
        )
        .map_err(|e| EngineError::Plugin(e.to_string()))?;

        globals
            .set("host", host)
            .map_err(|e| EngineError::Plugin(e.to_string()))?;

        // 加载策略源码
        ctx.eval::<Value, _>(js_source)
            .map_err(|e| EngineError::Plugin(format!("JS evaluation error: {}", e)))?;

        Ok(())
    }

    /// # Summary
    /// 调用 JS 的 onCandle 函数并解析结果。
    ///
    /// # Logic
    /// 1. 从全局获取 `onCandle` 函数引用。
    /// 2. 将 K 线 JSON 字符串作为参数传入。
    /// 3. 解析返回值为 JSON 字符串并反序列化为 `Option<Signal>`。
    fn call_on_candle(
        ctx: &rquickjs::Ctx<'_>,
        candle_json: &str,
    ) -> Result<Option<Signal>, EngineError> {
        let globals = ctx.globals();

        let on_candle: Function = globals
            .get("onCandle")
            .map_err(|e| EngineError::Plugin(format!("onCandle function not found: {}", e)))?;

        let result: String = on_candle
            .call((candle_json,))
            .map_err(|e| EngineError::Plugin(format!("onCandle execution error: {}", e)))?;

        // 解析返回值
        let signal: Option<Signal> = serde_json::from_str(&result)
            .map_err(|e| EngineError::Plugin(format!("Signal deserialization error: {}", e)))?;

        Ok(signal)
    }
}
