use async_trait::async_trait;
use okane_core::config::UserNotifyConfig;
use okane_core::notify::error::NotifyError;
use okane_core::notify::port::{Notifier, NotifierFactory};
use okane_core::store::port::SystemStore;
use std::sync::Arc;

/// # Summary
/// `NotifierFactory` 的默认实现。
/// 根据用户 ID 从数据库查询通知配置，动态创建对应的 Notifier 实例。
///
/// # Invariants
/// - 每次调用为指定用户创建独立的 Notifier。
/// - 如果用户未配置通知或配置为 "none"，返回 None。
pub struct DefaultNotifierFactory {
    store: Arc<dyn SystemStore>,
}

impl DefaultNotifierFactory {
    /// # Summary
    /// 创建工厂实例。
    ///
    /// # Arguments
    /// * `store` - 系统存储接口，用于查询用户的通知配置。
    pub fn new(store: Arc<dyn SystemStore>) -> Self {
        Self { store }
    }

    /// # Summary
    /// 根据用户通知配置创建 Notifier 实例。
    fn create_from_config(config: &UserNotifyConfig) -> Result<Option<Arc<dyn Notifier>>, NotifyError> {
        match config.channel.as_str() {
            "telegram" => {
                if config.telegram.bot_token.is_empty() || config.telegram.chat_id.is_empty() {
                    return Err(NotifyError::Config("Telegram bot_token and chat_id are required".to_string()));
                }
                Ok(Some(Arc::new(crate::telegram::TelegramNotifier::new(
                    config.telegram.bot_token.clone(),
                    config.telegram.chat_id.clone(),
                ))))
            }
            "email" => {
                let n = crate::email::EmailNotifier::new(
                    &config.email.smtp_host,
                    &config.email.smtp_user,
                    &config.email.smtp_pass,
                    &config.email.from,
                    &config.email.to,
                )?;
                Ok(Some(Arc::new(n)))
            }
            "none" | "" => Ok(None),
            other => Err(NotifyError::Config(format!("Unknown notify channel: {}", other))),
        }
    }
}

#[async_trait]
impl NotifierFactory for DefaultNotifierFactory {
    async fn create_for_user(&self, user_id: &str) -> Result<Option<Arc<dyn Notifier>>, NotifyError> {
        let config = self.store.get_user_notify_config(user_id).await
            .map_err(|e| NotifyError::Config(format!("Failed to query user notify config: {}", e)))?;

        match config {
            Some(cfg) => Self::create_from_config(&cfg),
            None => Ok(None), // 用户未配置通知
        }
    }
}
