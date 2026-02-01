use async_trait::async_trait;
use okane_core::notify::error::NotifyError;
use okane_core::notify::port::Notifier;
use reqwest;
use serde::Serialize;

/// # Summary
/// A notifier implementation that sends messages via Telegram Bot API.
///
/// # Invariants
/// * `bot_token` must be valid.
/// * `chat_id` must be accessible by the bot.
pub struct TelegramNotifier {
    /// The Bot API token.
    bot_token: String,
    /// The target Chat ID.
    chat_id: String,
    /// The HTTP client used for requests.
    client: reqwest::Client,
}

/// # Summary
/// Payload structure for Telegram `sendMessage` API.
#[derive(Serialize)]
struct TelegramMessage {
    chat_id: String,
    text: String,
    parse_mode: String,
}

impl TelegramNotifier {
    /// # Summary
    /// Creates a new `TelegramNotifier`.
    ///
    /// # Invariants
    /// * None.
    ///
    /// # Logic
    /// Initializes the struct with provided credentials and a default HTTP client.
    ///
    /// # Arguments
    /// * `bot_token` - The Telegram Bot API token.
    /// * `chat_id` - The target chat ID to send messages to.
    ///
    /// # Returns
    /// * A new instance of `TelegramNotifier`.
    pub fn new(bot_token: String, chat_id: String) -> Self {
        Self {
            bot_token,
            chat_id,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl Notifier for TelegramNotifier {
    /// # Summary
    /// Sends a notification to the configured Telegram chat.
    ///
    /// # Invariants
    /// * None.
    ///
    /// # Logic
    /// 1. Constructs the Telegram API URL.
    /// 2. Formats the message with a bold subject and the content.
    /// 3. Sends a POST request to the Telegram API.
    /// 4. Checks the response status and returns success or failure.
    ///
    /// # Arguments
    /// * `subject` - The subject of the notification (formatted as bold).
    /// * `content` - The main content of the notification.
    ///
    /// # Returns
    /// * `Ok(())` if the message was sent successfully.
    /// * `Err(NotifyError)` if a network error occurs or the API returns a non-success status.
    async fn notify(&self, subject: &str, content: &str) -> Result<(), NotifyError> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.bot_token);
        // Simple formatting: Bold subject + newline + content
        let text = format!("*{}*\n{}", subject, content);

        let payload = TelegramMessage {
            chat_id: self.chat_id.clone(),
            text,
            parse_mode: "Markdown".to_string(),
        };

        let response = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| NotifyError::Network(e.to_string()))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(NotifyError::Platform(format!(
                "Telegram API error: {}",
                error_text
            )));
        }

        Ok(())
    }
}
