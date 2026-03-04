use serde::{Deserialize, Serialize};

/// 全局应用配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub jwt_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub data_dir: String,
}

/// Telegram Bot 推送配置
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelegramConfig {
    /// Telegram Bot API Token
    #[serde(default)]
    pub bot_token: String,
    /// 目标 Chat ID
    #[serde(default)]
    pub chat_id: String,
}

/// Email SMTP 推送配置
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmailConfig {
    /// SMTP 服务器地址
    #[serde(default)]
    pub smtp_host: String,
    /// SMTP 用户名
    #[serde(default)]
    pub smtp_user: String,
    /// SMTP 密码
    #[serde(default)]
    pub smtp_pass: String,
    /// 发件人
    #[serde(default)]
    pub from: String,
    /// 收件人
    #[serde(default)]
    pub to: String,
}

/// # Summary
/// 用户级通知配置实体，存储在数据库中，每个用户独立配置。
///
/// # Invariants
/// - `channel` 仅允许 "none" | "telegram" | "email"。
/// - 选择了某渠道时，对应的子配置必须有效。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserNotifyConfig {
    /// 通知渠道: "none" | "telegram" | "email"
    pub channel: String,
    /// Telegram 推送配置
    pub telegram: TelegramConfig,
    /// Email 推送配置
    pub email: EmailConfig,
}

impl Default for UserNotifyConfig {
    fn default() -> Self {
        Self {
            channel: "none".to_string(),
            telegram: TelegramConfig::default(),
            email: EmailConfig::default(),
        }
    }
}

impl AppConfig {
    /// # Summary
    /// 校验配置合法性与安全性。
    /// 
    /// # Logic
    /// 如果 JWT 密钥仍为默认值，在非测试环境下发出警告。
    pub fn validate(&self) {
        if self.server.jwt_secret == "YOUR_SUPER_SECRET_KEY" {
            tracing::warn!("⚠️  SECURITY WARNING: Using default JWT secret key. This is highly discouraged for production use.");
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 8080,
                jwt_secret: "YOUR_SUPER_SECRET_KEY".to_string(),
            },
            database: DatabaseConfig {
                data_dir: "data".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.jwt_secret, "YOUR_SUPER_SECRET_KEY");
        assert_eq!(config.database.data_dir, "data");
    }
}
