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
    fn create_from_config(
        config: &UserNotifyConfig,
    ) -> Result<Option<Arc<dyn Notifier>>, NotifyError> {
        match config.channel.as_str() {
            "telegram" => {
                if config.telegram.bot_token.is_empty() || config.telegram.chat_id.is_empty() {
                    return Err(NotifyError::Config(
                        "Telegram bot_token and chat_id are required".to_string(),
                    ));
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
            other => Err(NotifyError::Config(format!(
                "Unknown notify channel: {}",
                other
            ))),
        }
    }
}

#[async_trait]
impl NotifierFactory for DefaultNotifierFactory {
    async fn create_for_user(
        &self,
        user_id: &str,
    ) -> Result<Option<Arc<dyn Notifier>>, NotifyError> {
        let config = self
            .store
            .get_user_notify_config(user_id)
            .await
            .map_err(|e| {
                NotifyError::Config(format!("Failed to query user notify config: {}", e))
            })?;

        match config {
            Some(cfg) => Self::create_from_config(&cfg),
            None => Ok(None), // 用户未配置通知
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use okane_core::config::{EmailConfig, TelegramConfig, UserNotifyConfig};
    use okane_core::store::error::StoreError;
    use okane_core::store::port::{Position, StockMetadata, SystemStore, User, UserSession};

    fn init_rustls() {
        okane_core::common::install_rustls_crypto_provider();
    }

    struct MockSystemStore {
        config: Option<UserNotifyConfig>,
    }

    fn unsupported_store_call() -> StoreError {
        StoreError::Unknown("unsupported mock store call".to_string())
    }

    #[async_trait]
    impl SystemStore for MockSystemStore {
        async fn get_user_notify_config(
            &self,
            _user_id: &str,
        ) -> Result<Option<UserNotifyConfig>, StoreError> {
            Ok(self.config.clone())
        }

        // --- Unimplemented methods for mock ---
        async fn get_user(&self, _: &str) -> Result<Option<User>, StoreError> {
            Err(unsupported_store_call())
        }
        async fn get_account_owner(&self, _: &str) -> Result<Option<String>, StoreError> {
            Err(unsupported_store_call())
        }
        async fn bind_account(&self, _: &str, _: &str) -> Result<(), StoreError> {
            Err(unsupported_store_call())
        }
        async fn get_user_accounts(&self, _: &str) -> Result<Vec<String>, StoreError> {
            Err(unsupported_store_call())
        }
        async fn save_user(&self, _: &User) -> Result<(), StoreError> {
            Err(unsupported_store_call())
        }
        async fn get_watchlist(&self, _: &str) -> Result<Vec<String>, StoreError> {
            Err(unsupported_store_call())
        }
        async fn add_to_watchlist(&self, _: &str, _: &str) -> Result<(), StoreError> {
            Err(unsupported_store_call())
        }
        async fn remove_from_watchlist(&self, _: &str, _: &str) -> Result<(), StoreError> {
            Err(unsupported_store_call())
        }
        async fn get_positions(&self, _: &str) -> Result<Vec<Position>, StoreError> {
            Err(unsupported_store_call())
        }
        async fn update_position(&self, _: &str, _: &Position) -> Result<(), StoreError> {
            Err(unsupported_store_call())
        }
        async fn search_stocks(&self, _: &str) -> Result<Vec<StockMetadata>, StoreError> {
            Err(unsupported_store_call())
        }
        async fn save_stock_metadata(&self, _: &StockMetadata) -> Result<(), StoreError> {
            Err(unsupported_store_call())
        }
        async fn get_setting(&self, _: &str) -> Result<Option<String>, StoreError> {
            Err(unsupported_store_call())
        }
        async fn set_setting(&self, _: &str, _: &str) -> Result<(), StoreError> {
            Err(unsupported_store_call())
        }
        async fn save_user_notify_config(
            &self,
            _: &str,
            _: &UserNotifyConfig,
        ) -> Result<(), StoreError> {
            Err(unsupported_store_call())
        }
        async fn save_session(&self, _: &UserSession) -> Result<(), StoreError> {
            Err(unsupported_store_call())
        }
        async fn get_session(&self, _: &str) -> Result<Option<UserSession>, StoreError> {
            Err(unsupported_store_call())
        }
        async fn get_session_by_client(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<UserSession>, StoreError> {
            Err(unsupported_store_call())
        }
        async fn revoke_session(&self, _: &str) -> Result<(), StoreError> {
            Err(unsupported_store_call())
        }
        async fn revoke_all_user_sessions(&self, _: &str) -> Result<(), StoreError> {
            Err(unsupported_store_call())
        }
        async fn delete_expired_sessions(&self) -> Result<(), StoreError> {
            Err(unsupported_store_call())
        }
        async fn list_active_sessions(&self) -> Result<Vec<UserSession>, StoreError> {
            Err(unsupported_store_call())
        }
    }

    #[tokio::test]
    async fn test_create_telegram_notifier() -> anyhow::Result<()> {
        init_rustls();

        let config = UserNotifyConfig {
            channel: "telegram".to_string(),
            telegram: TelegramConfig {
                bot_token: "123:abc".to_string(),
                chat_id: "987".to_string(),
            },
            email: EmailConfig::default(),
        };
        let store = Arc::new(MockSystemStore {
            config: Some(config),
        });
        let factory = DefaultNotifierFactory::new(store);

        let notifier = factory.create_for_user("user1").await?;
        assert!(notifier.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn test_create_email_notifier() -> anyhow::Result<()> {
        init_rustls();

        let config = UserNotifyConfig {
            channel: "email".to_string(),
            telegram: TelegramConfig::default(),
            email: EmailConfig {
                smtp_host: "smtp.example.com".to_string(),
                smtp_user: "user@example.com".to_string(),
                smtp_pass: "pass".to_string(),
                from: "user@example.com".to_string(),
                to: "recipient@example.com".to_string(),
            },
        };
        let store = Arc::new(MockSystemStore {
            config: Some(config),
        });
        let factory = DefaultNotifierFactory::new(store);

        let notifier = factory.create_for_user("user1").await?;
        assert!(notifier.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn test_create_none_notifier() -> anyhow::Result<()> {
        let config = UserNotifyConfig::default();
        let store = Arc::new(MockSystemStore {
            config: Some(config),
        });
        let factory = DefaultNotifierFactory::new(store);

        let notifier = factory.create_for_user("user1").await?;
        assert!(notifier.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_create_unknown_channel() -> anyhow::Result<()> {
        let config = UserNotifyConfig {
            channel: "whatsapp".to_string(),
            ..UserNotifyConfig::default()
        };
        let store = Arc::new(MockSystemStore {
            config: Some(config),
        });
        let factory = DefaultNotifierFactory::new(store);

        let result = factory.create_for_user("user1").await;
        assert!(result.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn test_create_for_user_store_error() -> anyhow::Result<()> {
        struct ErrorStore;

        #[async_trait]
        impl SystemStore for ErrorStore {
            async fn get_user(&self, _: &str) -> Result<Option<User>, StoreError> {
                Err(StoreError::Database("db down".to_string()))
            }
            async fn get_account_owner(&self, _: &str) -> Result<Option<String>, StoreError> {
                Err(unsupported_store_call())
            }
            async fn bind_account(&self, _: &str, _: &str) -> Result<(), StoreError> {
                Err(unsupported_store_call())
            }
            async fn get_user_accounts(&self, _: &str) -> Result<Vec<String>, StoreError> {
                Err(unsupported_store_call())
            }
            async fn save_user(&self, _: &User) -> Result<(), StoreError> {
                Err(unsupported_store_call())
            }
            async fn get_watchlist(&self, _: &str) -> Result<Vec<String>, StoreError> {
                Err(unsupported_store_call())
            }
            async fn add_to_watchlist(&self, _: &str, _: &str) -> Result<(), StoreError> {
                Err(unsupported_store_call())
            }
            async fn remove_from_watchlist(&self, _: &str, _: &str) -> Result<(), StoreError> {
                Err(unsupported_store_call())
            }
            async fn get_positions(&self, _: &str) -> Result<Vec<Position>, StoreError> {
                Err(unsupported_store_call())
            }
            async fn update_position(&self, _: &str, _: &Position) -> Result<(), StoreError> {
                Err(unsupported_store_call())
            }
            async fn search_stocks(&self, _: &str) -> Result<Vec<StockMetadata>, StoreError> {
                Err(unsupported_store_call())
            }
            async fn save_stock_metadata(&self, _: &StockMetadata) -> Result<(), StoreError> {
                Err(unsupported_store_call())
            }
            async fn get_setting(&self, _: &str) -> Result<Option<String>, StoreError> {
                Err(unsupported_store_call())
            }
            async fn set_setting(&self, _: &str, _: &str) -> Result<(), StoreError> {
                Err(unsupported_store_call())
            }
            async fn save_user_notify_config(
                &self,
                _: &str,
                _: &UserNotifyConfig,
            ) -> Result<(), StoreError> {
                Err(unsupported_store_call())
            }
            async fn get_user_notify_config(
                &self,
                _: &str,
            ) -> Result<Option<UserNotifyConfig>, StoreError> {
                Err(StoreError::Database("db down".to_string()))
            }
            async fn save_session(&self, _: &UserSession) -> Result<(), StoreError> {
                Err(unsupported_store_call())
            }
            async fn get_session(&self, _: &str) -> Result<Option<UserSession>, StoreError> {
                Err(unsupported_store_call())
            }
            async fn get_session_by_client(
                &self,
                _: &str,
                _: &str,
            ) -> Result<Option<UserSession>, StoreError> {
                Err(unsupported_store_call())
            }
            async fn revoke_session(&self, _: &str) -> Result<(), StoreError> {
                Err(unsupported_store_call())
            }
            async fn revoke_all_user_sessions(&self, _: &str) -> Result<(), StoreError> {
                Err(unsupported_store_call())
            }
            async fn delete_expired_sessions(&self) -> Result<(), StoreError> {
                Err(unsupported_store_call())
            }
            async fn list_active_sessions(&self) -> Result<Vec<UserSession>, StoreError> {
                Err(unsupported_store_call())
            }
        }

        let factory = DefaultNotifierFactory::new(Arc::new(ErrorStore));
        let err = factory
            .create_for_user("user1")
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected store error"))?;
        assert!(err.to_string().to_lowercase().contains("failed to query"));
        Ok(())
    }

    #[tokio::test]
    async fn test_create_telegram_missing_config() -> anyhow::Result<()> {
        let config = UserNotifyConfig {
            channel: "telegram".to_string(),
            telegram: TelegramConfig {
                bot_token: "".to_string(), // Missing token
                chat_id: "987".to_string(),
            },
            email: EmailConfig::default(),
        };
        let store = Arc::new(MockSystemStore {
            config: Some(config),
        });
        let factory = DefaultNotifierFactory::new(store);

        let result = factory.create_for_user("user1").await;
        assert!(result.is_err());
        assert!(
            format!("{:?}", result.err()).contains("Telegram bot_token and chat_id are required")
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_create_for_user_not_found() -> anyhow::Result<()> {
        let store = Arc::new(MockSystemStore { config: None });
        let factory = DefaultNotifierFactory::new(store);

        let notifier = factory.create_for_user("unknown_user").await?;
        assert!(notifier.is_none());
        Ok(())
    }
}
