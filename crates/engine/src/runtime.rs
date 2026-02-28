use okane_core::common::time::TimeProvider;

use okane_core::common::TimeFrame;
use okane_core::engine::entity::Signal;
use okane_core::engine::error::EngineError;
use okane_core::engine::port::SignalHandler;
use okane_core::market::port::Market;
use std::sync::Arc;

/// # Summary
/// 策略来源枚举，区分 JS 和 WASM 策略。
///
/// # Invariants
/// - `JavaScript` 变体必须包含合法的 ES2020 源码。
/// - `Wasm` 变体必须包含合法的 WASM 模块字节。
pub enum StrategySource {
    /// JS 源码直接执行（通过 QuickJS RuntimeEngine）
    JavaScript(String),
    /// 编译后的 WASM 字节码（通过 wasmtime RuntimeEngine）
    Wasm(Vec<u8>),
}

/// # Summary
/// 插件执行上下文，提供策略运行所需的宿主能力。
///
/// # Invariants
/// - `market` 引用在上下文生命周期内有效。
/// - `current_time` 在每次 K 线到达时更新。
pub struct PluginContext {
    /// 市场数据访问端口
    pub market: Arc<dyn Market>,
    /// 交易指令下发端口
    pub trade_port: Arc<dyn okane_core::trade::port::TradePort>,
    /// 绑定的交易账户 ID
    pub account_id: String,
    /// 当前挂载的时钟源，保证回测和实盘的时间分轨
    pub time_provider: Arc<dyn TimeProvider>,
}

/// # Summary
/// 引擎基础设施，封装市场访问和信号分发的公共能力。
///
/// # Invariants
/// - `handlers` 中的每个处理器必须实现 `SignalHandler`。
/// - 信号分发顺序与注册顺序一致。
pub struct EngineBase {
    /// 市场数据访问端口
    pub market: Arc<dyn Market>,
    /// 已注册的信号处理器列表
    pub handlers: Vec<Box<dyn SignalHandler>>,
}

impl EngineBase {
    /// # Summary
    /// 创建引擎基础设施实例。
    ///
    /// # Arguments
    /// * `market`: 市场数据驱动接口。
    ///
    /// # Returns
    /// * `Self` - 初始化后的实例。
    pub fn new(market: Arc<dyn Market>) -> Self {
        Self {
            market,
            handlers: Vec::new(),
        }
    }

    /// # Summary
    /// 注册信号处理器。
    ///
    /// # Arguments
    /// * `handler`: 信号处理钩子实现。
    pub fn register_handler(&mut self, handler: Box<dyn SignalHandler>) {
        self.handlers.push(handler);
    }

    /// # Summary
    /// 分发信号到所有匹配的处理器。
    ///
    /// # Logic
    /// 1. 遍历所有已注册的处理器。
    /// 2. 若匹配则调用其 handle 方法。
    ///
    /// # Arguments
    /// * `signal`: 策略产生的信号。
    ///
    /// # Returns
    /// * `Result<(), EngineError>` - 分发结果。
    pub async fn dispatch_signal(&self, signal: Signal) -> Result<(), EngineError> {
        for handler in &self.handlers {
            if handler.matches(&signal) {
                handler.handle(signal.clone()).await?;
            }
        }
        Ok(())
    }

    /// # Summary
    /// 创建针对特定证券和周期的 K 线订阅流。
    ///
    /// # Arguments
    /// * `symbol`: 证券代码。
    /// * `timeframe`: K 线时间周期。
    ///
    /// # Returns
    /// * 成功返回 K 线流和 Stock 聚合根，失败返回 EngineError。
    pub async fn subscribe(
        &self,
        symbol: &str,
        timeframe: TimeFrame,
    ) -> Result<okane_core::market::port::CandleStream, EngineError> {
        let stock = self
            .market
            .get_stock(symbol)
            .await
            .map_err(|e| EngineError::Market(e.to_string()))?;
        Ok(stock.subscribe(timeframe))
    }
}
