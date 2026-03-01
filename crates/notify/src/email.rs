use async_trait::async_trait;
use lettre::message::{Message, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use okane_core::notify::error::NotifyError;
use okane_core::notify::port::Notifier;

/// # Summary
/// A notifier implementation that sends messages via SMTP (e.g., Gmail, QQ Mail).
///
/// # Invariants
/// - Requires valid SMTP credentials and server configuration.
/// - The `AsyncSmtpTransport` is reused for multiple notifications.
pub struct EmailNotifier {
    /// The asynchronous SMTP transport.
    mailer: AsyncSmtpTransport<Tokio1Executor>,
    /// The sender's email address.
    from: String,
    /// The recipient's email address.
    to: String,
}

impl EmailNotifier {
    /// # Summary
    /// Creates a new `EmailNotifier`.
    ///
    /// # Logic
    /// 1. Sets up the SMTP credentials.
    /// 2. Configures the relay transport with TLS and authentication.
    ///
    /// # Arguments
    /// * `host` - The SMTP server host (e.g., "smtp.gmail.com").
    /// * `user` - The SMTP username (email address).
    /// * `password` - The SMTP password or app-specific password.
    /// * `from` - The sender's email address.
    /// * `to` - The recipient's email address.
    ///
    /// # Returns
    /// * A new instance of `EmailNotifier` or `NotifyError`.
    pub fn new(host: &str, user: &str, pass: &str, from: &str, to: &str) -> Result<Self, NotifyError> {
        let creds = Credentials::new(user.to_string(), pass.to_string());

        // Use default submission port 587 with STARTTLS
        let mailer = AsyncSmtpTransport::<Tokio1Executor>::relay(host)
            .map_err(|e| NotifyError::Config(format!("Invalid SMTP host: {}", e)))?
            .credentials(creds)
            .build();

        Ok(Self {
            mailer,
            from: from.to_string(),
            to: to.to_string(),
        })
    }
}

#[async_trait]
impl Notifier for EmailNotifier {
    /// # Summary
    /// Sends a notification email.
    ///
    /// # Logic
    /// 1. Builds an email message with the subject and content.
    /// 2. Sets the Content-Type to plain text.
    /// 3. Sends the email using the configured SMTP transport.
    ///
    /// # Arguments
    /// * `subject` - The subject line of the email.
    /// * `content` - The body content of the email.
    ///
    /// # Returns
    /// * `Ok(())` if the email was successfully sent.
    /// * `Err(NotifyError)` if a network or SMTP error occurs.
    async fn notify(&self, subject: &str, content: &str) -> Result<(), NotifyError> {
        let email = Message::builder()
            .from(
                self.from
                    .parse()
                    .map_err(|e| NotifyError::Config(format!("Invalid from address: {}", e)))?,
            )
            .to(self
                .to
                .parse()
                .map_err(|e| NotifyError::Config(format!("Invalid to address: {}", e)))?)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(content.to_string())
            .map_err(|e| NotifyError::Platform(format!("Failed to build email: {}", e)))?;

        self.mailer
            .send(email)
            .await
            .map_err(|e| NotifyError::Network(format!("SMTP error: {}", e)))?;

        Ok(())
    }
}
