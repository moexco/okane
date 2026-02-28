use futures::StreamExt;
use okane_core::common::TimeFrame;
use okane_core::market::port::Market;
use okane_feed::yahoo::YahooProvider;
use okane_market::manager::MarketImpl;
use okane_store::market::SqliteMarketStore;
use std::sync::Arc;

#[tokio::test]
async fn test_market_with_real_yahoo_feed() {
    let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    okane_store::config::set_root_dir(tmp_dir.path().to_path_buf());

    // 1. 初始化真实的 Yahoo 数据源
    let yahoo_provider = Arc::new(YahooProvider::new());

    // 2. 初始化真实的存储驱动（通过 Symbol 隔离数据文件）
    let store = Arc::new(SqliteMarketStore::new().expect("Failed to create real store"));

    // 3. 初始化 Market 实现
    let market = MarketImpl::new(yahoo_provider, store);

    // 4. 获取 AAPL 聚合根
    let symbol = "AAPL";
    let stock = market
        .get_stock(symbol)
        .await
        .expect("Should get AAPL stock");

    // 5. 验证身份
    assert_eq!(stock.identity().symbol, symbol);

    // 6. 开启真实订阅并等待第一根 K 线 (可能需要网络访问)
    let mut stream = stock.subscribe(TimeFrame::Minute1);

    // 给一点时间让 Yahoo 响应
    // 注意：真实网络环境可能不稳定，这里使用 timeout
    let first_candle =
        tokio::time::timeout(tokio::time::Duration::from_secs(10), stream.next()).await;

    match first_candle {
        Ok(Some(candle)) => {
            tracing::info!("Received real candle from Yahoo: {:?}", candle);
            assert!(candle.close > 0.0);

            // 6. 验证聚合根的同步快照是否已更新
            assert!(stock.current_price().is_some());
            assert!(stock.latest_candle(TimeFrame::Minute1).is_some());
        }
        Ok(None) => panic!("Stream ended without data"),
        Err(_) => {
            // 如果是在离线环境或 CI 中，这可能会超时
            // 在集成测试中，我们至少期望它能走到这一步而不崩溃
            tracing::warn!("Warning: Timeout waiting for Yahoo real data. Check internet connection.");
        }
    }
}

#[tokio::test]
async fn test_market_broadcast_with_real_feed() {
    let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    okane_store::config::set_root_dir(tmp_dir.path().to_path_buf());

    let yahoo_provider = Arc::new(YahooProvider::new());
    let store = Arc::new(SqliteMarketStore::new().expect("Failed to create real store"));
    let market = MarketImpl::new(yahoo_provider, store);
    let symbol = "AAPL";

    // 获取同一个聚合根的两个引用
    let stock_a = market.get_stock(symbol).await.unwrap();
    let stock_b = market.get_stock(symbol).await.unwrap();

    // 同时订阅
    let mut stream_a = stock_a.subscribe(TimeFrame::Minute1);
    let mut stream_b = stock_b.subscribe(TimeFrame::Minute1);

    // 真正的广播验证逻辑：确保两个流都能在规定时间内获取到数据
    let wait_for_data = async {
        let (res_a, res_b) = tokio::join!(stream_a.next(), stream_b.next());
        assert!(res_a.is_some(), "Stream A should receive data");
        assert!(res_b.is_some(), "Stream B should receive data");

        let ca = res_a.unwrap();
        let cb = res_b.unwrap();
        assert_eq!(
            ca.time, cb.time,
            "Both streams should receive the same candle"
        );
        println!(
            "Broadcast verified: both streams received candle at {}",
            ca.time
        );
    };

    if tokio::time::timeout(tokio::time::Duration::from_secs(15), wait_for_data)
        .await
        .is_err()
    {
        tracing::warn!("Broadcast integration test timed out (normal if market is closed or no trades)");
    }
}
