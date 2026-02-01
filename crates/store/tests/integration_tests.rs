use chrono::{TimeZone, Utc};
use okane_core::common::{Stock, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::store::port::{MarketStore, Position, StockMetadata, SystemStore, User};
use okane_store::config::set_root_dir;
use okane_store::market::SqliteMarketStore;
use okane_store::system::SqliteSystemStore;
use tempfile::tempdir;

#[tokio::test]
async fn test_store_full_integration() {
    // 1. 初始化临时测试环境
    let tmp_dir = tempdir().expect("Failed to create temp dir");
    let root_path = tmp_dir.path().to_path_buf();
    set_root_dir(root_path.clone());

    // 2. 测试 SqliteSystemStore
    let system_store = SqliteSystemStore::new()
        .await
        .expect("Failed to create system store");

    // 用户存取
    let user = User {
        id: "u1".to_string(),
        name: "Tester".to_string(),
        created_at: Utc::now(),
    };
    system_store.save_user(&user).await.unwrap();
    let saved_user = system_store.get_user("u1").await.unwrap().expect("User should exist");
    assert_eq!(saved_user.name, "Tester");

    // 自选股
    system_store.add_to_watchlist("u1", "AAPL").await.unwrap();
    let watchlist = system_store.get_watchlist("u1").await.unwrap();
    assert!(watchlist.contains(&"AAPL".to_string()));

    // 持仓
    let pos = Position {
        symbol: "AAPL".to_string(),
        quantity: 100.0,
        avg_price: 150.0,
        last_updated: Utc::now(),
    };
    system_store.update_position("u1", &pos).await.unwrap();
    let positions = system_store.get_positions("u1").await.unwrap();
    assert_eq!(positions[0].quantity, 100.0);

    // 股票搜索
    let meta = StockMetadata {
        symbol: "TSLA".to_string(),
        name: "Tesla Inc".to_string(),
        exchange: "NASDAQ".to_string(),
        sector: Some("Auto".to_string()),
        currency: "USD".to_string(),
    };
    system_store.save_stock_metadata(&meta).await.unwrap();
    let search_results = system_store.search_stocks("Tesla").await.unwrap();
    assert_eq!(search_results.len(), 1);

    // 3. 测试 SqliteMarketStore
    let market_store = SqliteMarketStore::new().expect("Failed to create market store");
    let stock = Stock {
        symbol: "AAPL".into(),
        exchange: Some("NASDAQ".into()),
    };
    let candles = vec![Candle {
        time: Utc.with_ymd_and_hms(2026, 2, 1, 10, 0, 0).unwrap(),
        open: 150.0,
        high: 155.0,
        low: 149.0,
        close: 152.0,
        adj_close: Some(152.0),
        volume: 10000.0,
        is_final: true,
    }];

    market_store.save_candles(&stock, TimeFrame::Day1, &candles).await.unwrap();

    // 验证物理路径 (应当在临时目录下)
    let db_file = root_path.join("market").join("AAPL_NASDAQ.db");
    assert!(db_file.exists());

    // 读取验证
    let start = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2026, 2, 1, 23, 59, 59).unwrap();
    let loaded = market_store.load_candles(&stock, TimeFrame::Day1, start, end).await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].close, 152.0);
    assert_eq!(loaded[0].adj_close, Some(152.0));
    assert!(loaded[0].is_final);
}
