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
