use crate::bridge::AsyncBridge;
use okane_core::common::time::TimeProvider;

use okane_core::common::TimeFrame;
use okane_core::engine::error::EngineError;
use okane_core::market::port::Market;
use okane_core::market::indicator::IndicatorService;
use okane_core::trade::port::{TradePort, AlgoOrderPort};
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
}

/// # Summary
/// 插件执行上下文，提供策略运行所需的宿主能力。
///
/// # Invariants
/// - `market` 引用在上下文生命周期内有效。
pub struct PluginContext {
    /// 市场数据访问端口
    pub market: Arc<dyn Market>,
    /// 交易指令下发端口
    /// 交易指令下发端口
    pub trade_port: Arc<dyn TradePort>,
    /// 算法单端口
    pub algo_port: Arc<dyn AlgoOrderPort>,
    /// 技术指标服务
    pub indicator_service: Arc<dyn IndicatorService>,
    /// 绑定的交易账户 ID
    pub account_id: String,
    /// 当前挂载的时钟源，保证回测和实盘的时间分轨
    pub time_provider: Arc<dyn TimeProvider>,
    /// 通知推送端口 (可选)
    pub notifier: Option<Arc<dyn okane_core::notify::port::Notifier>>,
    /// 策略日志记录器 (可选)
    pub logger: Option<Arc<dyn okane_core::strategy::port::StrategyLogger>>,
    /// Sync-async bridge for host function callbacks
    pub bridge: Arc<AsyncBridge>,
}

/// # Summary
/// 引擎基础设施，封装市场访问的公共能力。
pub struct EngineBase {
    /// 市场数据访问端口
    pub market: Arc<dyn Market>,
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
        Self { market }
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
        stock.subscribe(timeframe).map_err(|e| EngineError::Market(e.to_string()))
    }
}
