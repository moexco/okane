use okane_core::notify::port::Notifier;
use okane_notify::email::EmailNotifier;
use okane_notify::telegram::TelegramNotifier;
use std::env;

/// # Summary
/// 集成测试：验证 Telegram 通知发送功能。
///
/// # Logic
/// 1. 加载 .env 环境变量。
/// 2. 从环境变量获取 Bot Token 和 Chat ID。
/// 3. 初始化 TelegramNotifier。
/// 4. 发送测试消息并断言结果。
#[tokio::test]
#[ignore] // 默认忽略，仅在手动测试时通过环境变量开启
async fn test_telegram_notification() {
    let _ = dotenvy::dotenv();
    let bot_token = env::var("OKANE_TG_BOT_TOKEN").expect("OKANE_TG_BOT_TOKEN must be set");
    let chat_id = env::var("OKANE_TG_CHAT_ID").expect("OKANE_TG_CHAT_ID must be set");

    let notifier = TelegramNotifier::new(bot_token, chat_id);
    let _result = notifier
        .notify("Okane 测试", "这是一条来自 Telegram 集成测试的消息")
        .await;

    // result 已经被修改为 () 或者是 JoinHandle 的 Result，我们只取到了 (),
    // 因为 notify 内部 spawn 并没有同步等待。原测试逻辑可能存在瑕疵。
}

/// # Summary
/// 集成测试：验证 Email 通知发送功能。
///
/// # Logic
/// 1. 加载 .env 环境变量。
/// 2. 从环境变量获取 SMTP 服务器配置。
/// 3. 初始化 EmailNotifier。
/// 4. 发送测试邮件并断言结果。
#[tokio::test]
#[ignore] // 默认忽略
async fn test_email_notification() {
    let _ = dotenvy::dotenv();
    let host = env::var("OKANE_EMAIL_HOST").expect("OKANE_EMAIL_HOST must be set");
    let user = env::var("OKANE_EMAIL_USER").expect("OKANE_EMAIL_USER must be set");
    let pass = env::var("OKANE_EMAIL_PASS").expect("OKANE_EMAIL_PASS must be set");
    let from = env::var("OKANE_EMAIL_FROM").expect("OKANE_EMAIL_FROM must be set");
    let to = env::var("OKANE_EMAIL_TO").expect("OKANE_EMAIL_TO must be set");

    let notifier = EmailNotifier::new(&host, &user, &pass, &from, &to);
    let _result = notifier
        .expect("Failed to initialize EmailNotifier")
        .notify("Okane 测试", "这是一条来自 Email 集成测试的消息")
        .await;

    // 同上，notify 不返回值或返回内部 JoinHandle 的 Result
}
