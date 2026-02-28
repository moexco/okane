use crate::engine::entity::Signal;
use crate::engine::error::EngineError;
use async_trait::async_trait;

/// # Summary
/// 信号处理能力接口 (Port)。
/// 任何想要响应策略信号的组件（如下单器、通知器）都必须实现此接口并注册到 Engine。
///
/// # Invariants
/// - 实现类必须保证线程安全 (`Send` + `Sync`)。
#[async_trait]
pub trait SignalHandler: Send + Sync {
    /// # Summary
    /// 该处理器是否支持处理此特定的信号。
    ///
    /// # Logic
    /// 处理器通常基于 `SignalKind` 或 `symbol` 进行匹配。
    ///
    /// # Arguments
    /// * `signal`: 待检查的信号引用。
    ///
    /// # Returns
    /// * `bool` - 是否处理。
    fn matches(&self, signal: &Signal) -> bool;

    /// # Summary
    /// 执行具体的处理逻辑。
    ///
    /// # Logic
    /// 1. 解析信号内容。
    /// 2. 调用外部适配器（如 Telegram API 或 交易 API）。
    ///
    /// # Arguments
    /// * `signal`: 捕获到的信号所有权。
    ///
    /// # Returns
    /// * 成功返回 `Ok(())`，失败返回 `EngineError::Handler`。
    async fn handle(&self, signal: Signal) -> Result<(), EngineError>;
}

use crate::common::TimeFrame;
use crate::strategy::entity::EngineType;
use std::future::Future;
use std::pin::Pin;

/// # Summary
/// 策略执行的异步任务抽象，代表策略从启动到终止的生命周期。
pub type EngineFuture = Pin<Box<dyn Future<Output = Result<(), EngineError>> + Send>>;

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
    /// * `engine_type` - 目标引擎类型。
    /// * `symbol` - 目标证券代码。
    /// * `timeframe` - K 线时间周期。
    /// * `source` - 策略源码 (JS) 或字节码 (WASM)。
    /// * `handlers` - 信号处理器列表，由调用方组装后传入。
    ///
    /// # Returns
    /// * `Result<Pin<Box<dyn Future<...>>>>` - 可 spawn 的异步任务闭包。
    fn build(
        &self,
        engine_type: EngineType,
        symbol: String,
        timeframe: TimeFrame,
        source: Vec<u8>,
        handlers: Vec<Box<dyn SignalHandler>>,
    ) -> Result<EngineFuture, EngineError>;
}
