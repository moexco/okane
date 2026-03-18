use chrono::{TimeZone, Utc};
use okane_core::common::{Stock, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::store::port::{MarketStore, Position, StockMetadata, SystemStore, User};
use okane_store::config::set_root_dir;
use okane_store::market::SqliteMarketStore;
use okane_store::system::SqliteSystemStore;
use rust_decimal_macros::dec;
use tempfile::tempdir;

#[tokio::test]
async fn test_store_full_integration() -> anyhow::Result<()> {
    // 1. 初始化临时测试环境
    let tmp_dir = tempdir().map_err(|e| anyhow::anyhow!("Failed to create temp dir: {}", e))?;
    let root_path = tmp_dir.path().to_path_buf();
    set_root_dir(root_path.clone());

    // 2. 测试 SqliteSystemStore
    let system_store = SqliteSystemStore::new()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create system store: {}", e))?;

    // 用户存取
    let user = User {
        id: "u1".to_string(),
        name: "Tester".to_string(),
        password_hash: "dummy_hash".to_string(),
        role: okane_core::store::port::UserRole::Standard,
        force_password_change: false,
        created_at: Utc::now(),
    };
    system_store
        .save_user(&user)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let saved_user = system_store
        .get_user("u1")
        .await
        .map_err(|e| anyhow::anyhow!(e))?
        .ok_or_else(|| anyhow::anyhow!("User should exist"))?;
    assert_eq!(saved_user.name, "Tester");

    // 自选股
    system_store
        .add_to_watchlist("u1", "AAPL")
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let watchlist = system_store
        .get_watchlist("u1")
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    assert!(watchlist.contains(&"AAPL".to_string()));

    // 持仓
    let pos = Position {
        symbol: "AAPL".to_string(),
        quantity: dec!(100.0),
        avg_price: dec!(150.0),
        last_updated: Utc::now(),
    };
    system_store
        .update_position("u1", &pos)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let positions = system_store
        .get_positions("u1")
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    assert_eq!(positions[0].quantity, dec!(100.0));

    // 股票搜索
    let meta = StockMetadata {
        symbol: "TSLA".to_string(),
        name: "Tesla Inc".to_string(),
        exchange: "NASDAQ".to_string(),
        sector: Some("Auto".to_string()),
        currency: "USD".to_string(),
    };
    system_store
        .save_stock_metadata(&meta)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    let search_results = system_store
        .search_stocks("Tesla")
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    assert_eq!(search_results.len(), 1);

    // 3. 测试 SqliteMarketStore
    let market_store = SqliteMarketStore::new()
        .map_err(|e| anyhow::anyhow!("Failed to create market store: {}", e))?;
    let stock = Stock {
        symbol: "AAPL".into(),
        exchange: Some("NASDAQ".into()),
    };
    let candles = vec![Candle {
        time: Utc
            .with_ymd_and_hms(2026, 2, 1, 10, 0, 0)
            .single()
            .ok_or_else(|| anyhow::anyhow!("Invalid date"))?,
        open: dec!(150.0),
        high: dec!(155.0),
        low: dec!(149.0),
        close: dec!(152.0),
        adj_close: Some(dec!(152.0)),
        volume: dec!(10000.0),
        is_final: true,
    }];

    market_store
        .save_candles(&stock, TimeFrame::Day1, &candles)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // 验证物理路径 (应当在临时目录下)
    let db_file = root_path.join("market").join("AAPL_NASDAQ.db");
    assert!(db_file.exists());

    // 读取验证
    let start = Utc
        .with_ymd_and_hms(2026, 2, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| anyhow::anyhow!("Invalid date"))?;
    let end = Utc
        .with_ymd_and_hms(2026, 2, 1, 23, 59, 59)
        .single()
        .ok_or_else(|| anyhow::anyhow!("Invalid date"))?;
    let loaded = market_store
        .load_candles(&stock, TimeFrame::Day1, start, end)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].close, dec!(152.0));
    assert_eq!(loaded[0].adj_close, Some(dec!(152.0)));
    assert!(loaded[0].is_final);
    Ok(())
}
