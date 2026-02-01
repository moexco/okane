use crate::notify::error::NotifyError;
use async_trait::async_trait;

/// # Summary
/// 发送通知到外部系统的接口定义。
///
/// # Invariants
/// - 实现必须是 `Send` 和 `Sync` 以支持并发调用。
/// - `notify` 方法必须是异步的。
#[async_trait]
pub trait Notifier: Send + Sync {
    /// # Summary
    /// 发送带有主题和内容的通知。
    ///
    /// # Logic
    /// 1. 根据目标平台要求格式化消息。
    /// 2. 通过底层传输协议发送消息。
    /// 3. 返回成功或失败状态。
    ///
    /// # Arguments
    /// * `subject` - 通知标题或主题。
    /// * `content` - 通知的具体内容。
    ///
    /// # Returns
    /// * 成功返回 `Ok(())`。
    /// * 失败返回 `Err(NotifyError)`。
    async fn notify(&self, subject: &str, content: &str) -> Result<(), NotifyError>;
}
