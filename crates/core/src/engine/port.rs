use crate::engine::error::EngineError;

use crate::common::TimeFrame;
use crate::strategy::entity::EngineType;
use std::future::Future;
use std::pin::Pin;

/// # Summary
/// 策略执行的异步任务抽象，代表策略从启动到终止的生命周期。
pub type EngineFuture = Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send>>;

/// # Summary
/// 构建引擎任务的参数集合。
pub struct EngineBuildParams {
    pub engine_type: EngineType,
    pub symbol: String,
    pub account_id: String,
    pub timeframe: TimeFrame,
    pub source: Vec<u8>,
    pub trade_port: std::sync::Arc<dyn crate::trade::port::TradePort>,
    pub algo_port: std::sync::Arc<dyn crate::trade::port::AlgoOrderPort>,
    pub indicator_service: std::sync::Arc<dyn crate::market::indicator::IndicatorService>,
    pub time_provider: std::sync::Arc<dyn crate::common::time::TimeProvider>,
    /// 通知推送端口 (可选, 用于 host.notify 能力)
    pub notifier: Option<std::sync::Arc<dyn crate::notify::port::Notifier>>,
    /// 策略日志记录器 (可选)
    pub logger: Option<std::sync::Arc<dyn crate::strategy::port::StrategyLogger>>,
}

/// # Summary
/// 引擎构建接口。
/// 由 `crates/engine` 实现，通过 `crates/app` 注入到 `crates/manager`，
/// 使 manager 无需编译期依赖任何具体引擎实现。
///
/// # Invariants
/// - 实现类必须保证线程安全 (`Send` + `Sync`)。
/// - 返回的 Future 代表策略的完整执行生命周期，直到被外部中止或自然结束。
pub trait EngineBuilder: Send + Sync {
    /// # Summary
    /// 根据引擎类型和策略配置，构建一个可执行的策略运行任务。
    ///
    /// # Arguments
    /// * `params` - 策略执行所需的所有参数。
    ///
    /// # Returns
    /// * `Result<Pin<Box<dyn Future<...>>>>` - 可 spawn 的异步任务闭包。
    fn build(
        &self,
        params: EngineBuildParams,
    ) -> Result<EngineFuture, EngineError>;
}
