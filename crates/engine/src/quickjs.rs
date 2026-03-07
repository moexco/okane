use crate::bridge::AsyncBridge;
use crate::runtime::{EngineBase, PluginContext};
use futures::StreamExt;
use okane_core::common::TimeFrame;
use okane_core::common::time::TimeProvider;
use okane_core::engine::error::EngineError;
use okane_core::market::port::Market;
use okane_core::market::error::MarketError;
use okane_core::market::entity::Candle;
use rquickjs::{AsyncContext, AsyncRuntime, Function, Object, Value, async_with};
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};


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
    notifier: Option<Arc<dyn okane_core::notify::port::Notifier>>,
    bridge: Arc<AsyncBridge>,
}

const JS_MEM_LIMIT: usize = 32 * 1024 * 1024;
const JS_STACK_SIZE: usize = 1024 * 1024;

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
        notifier: Option<Arc<dyn okane_core::notify::port::Notifier>>,
    ) -> Result<Self, EngineError> {
        Ok(Self {
            base: EngineBase::new(market),
            trade_port,
            time_provider,
            notifier,
            bridge: Arc::new(AsyncBridge::new()?),
        })
    }

    /// # Summary
    /// 运行 JS 策略。
    ///
    /// # Logic
    /// 1. 创建 QuickJS AsyncRuntime 和 AsyncContext，配置内存与栈大小限制。
    /// 2. 在 JS 全局注入 `host` 对象，包含 log/now/fetchHistory 方法。
    /// 3. 加载并执行策略 JS 源码。
    /// 4. 订阅 K 线流，每次到达时序列化为 JSON 调用 JS 的 `onCandle` 函数。
    /// 5. 策略通过 host.* API 直接执行动作，onCandle 为 void。
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
        rt.set_memory_limit(JS_MEM_LIMIT).await;

        // 设置最大栈大小：1MB
        rt.set_max_stack_size(JS_STACK_SIZE).await;

        let ctx = AsyncContext::full(&rt)
            .await
            .map_err(|_| EngineError::Plugin("log set failed".to_string()))?;

        // 共享的插件上下文
        let plugin_ctx = Arc::new(Mutex::new(PluginContext {
            market: self.base.market.clone(),
            trade_port: self.trade_port.clone(),
            account_id: account_id.to_string(),
            time_provider: self.time_provider.clone(),
            notifier: self.notifier.clone(),
            bridge: self.bridge.clone(),
        }));

        // 在 JS 全局注入 host 对象并加载策略源码
        let js_source_owned = js_source.to_string();
        let plugin_ctx_clone = plugin_ctx.clone();

        let bridge_clone = self.bridge.clone();

        async_with!(ctx => |ctx| {
            if let Err(e) = Self::setup_host_and_load(&ctx, &js_source_owned, plugin_ctx_clone, bridge_clone) {
                error!("JsEngine: Failed to setup host or load strategy: {}", e);
            }
        })
        .await;

        // 订阅 K 线流
        let mut stream = self.base.subscribe(symbol, timeframe).await?;

        // 核心执行循环
        while let Some(candle) = stream.next().await {
            // 序列化 K 线数据
            let candle_json = serde_json::to_string(&candle)
                .map_err(|_| EngineError::Plugin("log set failed".to_string()))?;

            // 调用 JS 的 onCandle 函数 (void — 策略通过 host.* API 直接执行动作)
            let candle_json_clone = candle_json.clone();
            let result: Result<(), EngineError> = async_with!(ctx => |ctx| {
                Self::call_on_candle(&ctx, &candle_json_clone)
            })
            .await;

            if let Err(e) = result {
                error!("JsEngine: Strategy execution failed for {}: {}", symbol, e);
                return Err(e);
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
        bridge: Arc<AsyncBridge>,
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
                    3 => info!("JS [INFO]: {}", msg),
                    _ => debug!("JS [DEBUG]: {}", msg),
                }
            })
            .map_err(|_| EngineError::Plugin("log setup failed".to_string()))?,
        )
        .map_err(|_| EngineError::Plugin("log set failed".to_string()))?;

        // host.now() -> number (milliseconds)
        let ctx_for_now = plugin_ctx.clone();
        host.set(
            "now",
            Function::new(ctx.clone(), move || -> Result<i64, rquickjs::Error> {
                let ctx = ctx_for_now.lock().map_err(|_| rquickjs::Error::Exception)?;
                ctx.time_provider.now()
                    .map(|t: chrono::DateTime<chrono::Utc>| t.timestamp_millis())
                    .map_err(|_| rquickjs::Error::Exception)
            })
            .map_err(|_| EngineError::Plugin("log setup failed".to_string()))?,
        )
        .map_err(|_| EngineError::Plugin("log set failed".to_string()))?;

        // host.fetchHistory(symbol: string, tf: string, limit: number) -> string (JSON)
        let ctx_for_fetch = plugin_ctx.clone();
        let bridge_for_fetch = bridge.clone();
        host.set(
            "fetchHistory",
            Function::new(ctx.clone(), move |symbol: String, tf: String, limit: i32| -> Result<String, rquickjs::Error> {
                let ctx_mutex = ctx_for_fetch.lock().map_err(|_| rquickjs::Error::Exception)?;
                let end_at = ctx_mutex.time_provider.now().map_err(|_| rquickjs::Error::Exception)?;
                let market = ctx_mutex.market.clone();
                drop(ctx_mutex);

                let tf_parsed = match tf.parse::<TimeFrame>() {
                    Ok(t) => t,
                    Err(e) => return Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                };

                match bridge_for_fetch.call(async move {
                    let stock = market.get_stock(&symbol).await.map_err(|e: MarketError| e.to_string())?;
                    let end = end_at;
                    let duration = tf_parsed.duration() * (limit * 2);
                    let start = end - duration;

                    let h = stock
                        .fetch_history(tf_parsed, start, end)
                        .await
                        .map_err(|e: MarketError| e.to_string())?;
                    
                    let usize_limit = usize::try_from(limit).map_err(|e| format!("Invalid limit: {}", e))?;
                    Ok::<Vec<Candle>, String>(h.into_iter().rev().take(usize_limit).rev().collect::<Vec<_>>())
                }) {
                    Ok(Ok(candles)) => Ok(serde_json::to_string(&candles).map_err(|_| rquickjs::Error::Exception)?),
                    Ok(Err(e)) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                    Err(e) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                }
            })
            .map_err(|_| EngineError::Plugin("log setup failed".to_string()))?,
        )
        .map_err(|_| EngineError::Plugin("log set failed".to_string()))?;

        // host.notify(subject: string, content: string) -> string ("ok" | error)
        let ctx_for_notify = plugin_ctx.clone();
        let bridge_for_notify = bridge.clone();
        host.set(
            "notify",
            Function::new(ctx.clone(), move |subject: String, content: String| -> Result<String, rquickjs::Error> {
                let ctx_mutex = ctx_for_notify.lock().map_err(|_| rquickjs::Error::Exception)?;
                let notifier = ctx_mutex.notifier.clone();
                drop(ctx_mutex);

                match notifier {
                    Some(n) => {
                        match bridge_for_notify.call(async move {
                            n.notify(&subject, &content).await.map_err(|e| e.to_string())
                        }) {
                            Ok(Ok(())) => Ok("ok".to_string()),
                            Ok(Err(e)) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                            Err(e) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                        }
                    }
                    None => {
                        warn!("JS called host.notify but no notifier is configured");
                        Ok(serde_json::json!({"error": "notifier not configured"}).to_string())
                    }
                }
            })
            .map_err(|_| EngineError::Plugin("log setup failed".to_string()))?,
        )
        .map_err(|_| EngineError::Plugin("log set failed".to_string()))?;

        // host.buy(symbol: string, price: string | null, volume: string) -> string (OrderId | Error)
        let ctx_for_buy = plugin_ctx.clone();
        let bridge_for_buy = bridge.clone();
        host.set(
            "buy",
            Function::new(ctx.clone(), move |symbol: String, price: Option<String>, volume: String| -> Result<String, rquickjs::Error> {
                let ctx_mutex = ctx_for_buy.lock().map_err(|_| rquickjs::Error::Exception)?;
                let trade_port = ctx_mutex.trade_port.clone();
                let account_id = ctx_mutex.account_id.clone();
                drop(ctx_mutex);

                let req_price = match price {
                    Some(p) => Some(p.parse::<rust_decimal::Decimal>().map_err(|_| rquickjs::Error::Exception)?),
                    None => None,
                };
                let req_vol = volume.parse::<rust_decimal::Decimal>().map_err(|_| rquickjs::Error::Exception)?;
                
                let order = okane_core::trade::entity::Order::new(
                    okane_core::trade::entity::OrderId(uuid::Uuid::new_v4().to_string()),
                    okane_core::trade::entity::AccountId(account_id),
                    symbol,
                    okane_core::trade::entity::OrderDirection::Buy,
                    req_price,
                    req_vol,
                    0,
                );

                match bridge_for_buy.call(async move {
                    trade_port.submit_order(order).await.map_err(|e| e.to_string())
                }) {
                    Ok(Ok(oid)) => Ok(oid.0),
                    Ok(Err(e)) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                    Err(e) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                }
            })
            .map_err(|_| EngineError::Plugin("log setup failed".to_string()))?,
        )
        .map_err(|_| EngineError::Plugin("log set failed".to_string()))?;

        // host.sell(symbol: string, price: string | null, volume: string) -> string (OrderId | Error)
        let ctx_for_sell = plugin_ctx.clone();
        let bridge_for_sell = bridge.clone();
        host.set(
            "sell",
            Function::new(ctx.clone(), move |symbol: String, price: Option<String>, volume: String| -> Result<String, rquickjs::Error> {
                let ctx_mutex = ctx_for_sell.lock().map_err(|_| rquickjs::Error::Exception)?;
                let trade_port = ctx_mutex.trade_port.clone();
                let account_id = ctx_mutex.account_id.clone();
                drop(ctx_mutex);

                let req_price = match price {
                    Some(p) => Some(p.parse::<rust_decimal::Decimal>().map_err(|_| rquickjs::Error::Exception)?),
                    None => None,
                };
                let req_vol = volume.parse::<rust_decimal::Decimal>().map_err(|_| rquickjs::Error::Exception)?;
                
                let order = okane_core::trade::entity::Order::new(
                    okane_core::trade::entity::OrderId(uuid::Uuid::new_v4().to_string()),
                    okane_core::trade::entity::AccountId(account_id),
                    symbol,
                    okane_core::trade::entity::OrderDirection::Sell,
                    req_price,
                    req_vol,
                    0,
                );

                match bridge_for_sell.call(async move {
                    trade_port.submit_order(order).await.map_err(|e| e.to_string())
                }) {
                    Ok(Ok(oid)) => Ok(oid.0),
                    Ok(Err(e)) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                    Err(e) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                }
            })
            .map_err(|_| EngineError::Plugin("log setup failed".to_string()))?,
        )
        .map_err(|_| EngineError::Plugin("log set failed".to_string()))?;

        // host.getAccount() -> string (JSON AccountSnapshot)
        let ctx_for_get_account = plugin_ctx.clone();
        let bridge_for_get_account = bridge.clone();
        host.set(
            "getAccount",
            Function::new(ctx.clone(), move || -> Result<String, rquickjs::Error> {
                let ctx_mutex = ctx_for_get_account.lock().map_err(|_| rquickjs::Error::Exception)?;
                let trade_port = ctx_mutex.trade_port.clone();
                let account_id = ctx_mutex.account_id.clone();
                drop(ctx_mutex);

                match bridge_for_get_account.call(async move {
                    trade_port.get_account(okane_core::trade::entity::AccountId(account_id)).await.map_err(|e| e.to_string())
                }) {
                    Ok(Ok(snapshot)) => Ok(serde_json::to_string(&snapshot).map_err(|_| rquickjs::Error::Exception)?),
                    Ok(Err(e)) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                    Err(e) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                }
            })
            .map_err(|_| EngineError::Plugin("log setup failed".to_string()))?,
        )
        .map_err(|_| EngineError::Plugin("log set failed".to_string()))?;

        // host.getOrder(orderId: string) -> string (JSON Order | null)
        let ctx_for_get_order = plugin_ctx.clone();
        let bridge_for_get_order = bridge.clone();
        host.set(
            "getOrder",
            Function::new(ctx.clone(), move |order_id: String| -> Result<String, rquickjs::Error> {
                let ctx_mutex = ctx_for_get_order.lock().map_err(|_| rquickjs::Error::Exception)?;
                let trade_port = ctx_mutex.trade_port.clone();
                drop(ctx_mutex);

                match bridge_for_get_order.call(async move {
                    trade_port.get_order(&okane_core::trade::entity::OrderId(order_id)).await.map_err(|e| e.to_string())
                }) {
                    Ok(Ok(Some(order))) => Ok(serde_json::to_string(&order).map_err(|_| rquickjs::Error::Exception)?),
                    Ok(Ok(None)) => Ok("null".to_string()),
                    Ok(Err(e)) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                    Err(e) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                }
            })
            .map_err(|_| EngineError::Plugin("log setup failed".to_string()))?,
        )
        .map_err(|_| EngineError::Plugin("log set failed".to_string()))?;

        // host.cancelOrder(orderId: string) -> string ("ok" | error)
        let ctx_for_cancel = plugin_ctx.clone();
        let bridge_for_cancel = bridge.clone();
        host.set(
            "cancelOrder",
            Function::new(ctx.clone(), move |order_id: String| -> Result<String, rquickjs::Error> {
                let ctx_mutex = ctx_for_cancel.lock().map_err(|_| rquickjs::Error::Exception)?;
                let trade_port = ctx_mutex.trade_port.clone();
                drop(ctx_mutex);

                info!("host.cancelOrder: orderId={}", order_id);

                match bridge_for_cancel.call(async move {
                    trade_port.cancel_order(okane_core::trade::entity::OrderId(order_id)).await.map_err(|e| e.to_string())
                }) {
                    Ok(Ok(())) => Ok("ok".to_string()),
                    Ok(Err(e)) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                    Err(e) => Ok(serde_json::json!({"error": e.to_string()}).to_string()),
                }
            })
            .map_err(|_| EngineError::Plugin("log setup failed".to_string()))?,
        )
        .map_err(|_| EngineError::Plugin("log set failed".to_string()))?;

        globals
            .set("host", host)
            .map_err(|_| EngineError::Plugin("log set failed".to_string()))?;

        // 加载策略源码
        ctx.eval::<Value, _>(js_source)
            .map_err(|e| EngineError::Plugin(format!("JS evaluation error: {}", e)))?;

        Ok(())
    }

    /// # Summary
    /// 调用 JS 的 onCandle 函数。
    ///
    /// # Logic
    /// 1. 从全局获取 `onCandle` 函数引用。
    /// 2. 将 K 线 JSON 字符串作为参数传入。
    /// 3. 策略通过 host.* API 直接执行动作，返回值被忽略。
    fn call_on_candle(
        ctx: &rquickjs::Ctx<'_>,
        candle_json: &str,
    ) -> Result<(), EngineError> {
        let globals = ctx.globals();

        let on_candle: Function = globals
            .get("onCandle")
            .map_err(|e| EngineError::Plugin(format!("onCandle function not found: {}", e)))?;

        let _result: Value = on_candle
            .call((candle_json,))
            .map_err(|e| EngineError::Plugin(format!("onCandle execution error: {}", e)))?;

        Ok(())
    }
}
