use crate::notify::error::NotifyError;
use async_trait::async_trait;
use std::sync::Arc;

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

/// # Summary
/// 通知实例工厂，根据用户 ID 从数据库查询配置并创建对应的 Notifier 实例。
///
/// # Invariants
/// - 实现必须是 `Send` 和 `Sync` 以支持并发调用。
/// - 每次调用为指定用户创建独立的 Notifier（不同用户互不干扰）。
#[async_trait]
pub trait NotifierFactory: Send + Sync {
    /// # Summary
    /// 根据用户 ID 创建通知推送实例。
    ///
    /// # Arguments
    /// * `user_id` - 用户唯一标识。
    ///
    /// # Returns
    /// * `Ok(Some(notifier))` - 用户已配置通知, 返回对应实例。
    /// * `Ok(None)` - 用户未配置通知或配置为 "none"。
    /// * `Err(NotifyError)` - 查询或创建失败。
    async fn create_for_user(
        &self,
        user_id: &str,
    ) -> Result<Option<Arc<dyn Notifier>>, NotifyError>;
}
