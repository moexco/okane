use okane_core::trade::entity::AccountId;
use okane_store::account::SqliteAccountStore;
use rust_decimal_macros::dec;
use tokio::time::Instant;
use okane_core::trade::entity::{OrderDirection, Trade};
use okane_core::trade::port::AccountPort;

#[tokio::test]
async fn test_sqlite_account_high_concurrency() -> anyhow::Result<()> {
    let tmp_dir = tempfile::tempdir().map_err(|e| anyhow::anyhow!("Failed to create temp dir: {}", e))?;
    okane_store::config::set_root_dir(tmp_dir.path().to_path_buf());

    let store = SqliteAccountStore::new().map_err(|e| anyhow::anyhow!("Failed to create store: {}", e))?;
    
    // Create an isolated account for db test
    let test_acct_id = format!("TestTx_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| anyhow::anyhow!(e))?.as_micros());
    let acct = AccountId(test_acct_id.clone());

    // Deposit 1000 initial cash
    store.deposit(&acct, dec!(1000.0)).await.map_err(|e| anyhow::anyhow!(e))?;

    let mut handles = vec![];
    let store = std::sync::Arc::new(store);

    let start = Instant::now();
    // Simulate 50 concurrent buy orders freezing funds then partially executing
    for i in 0..50 {
        let store_clone = store.clone();
        let a_id = acct.clone();
        handles.push(tokio::spawn(async move {
            let req_funds = dec!(15.0); // Predict 15$ needed
            // 1. Freeze
            let res = store_clone.freeze_funds(&a_id, req_funds).await;
            if res.is_ok() {
                // 2. Execute trade for 14$ (saving 1$)
                let trade = Trade {
                    order_id: okane_core::trade::entity::OrderId(format!("O_{}", i)),
                    account_id: a_id.clone(),
                    symbol: "AAPL".to_string(),
                    direction: OrderDirection::Buy,
                    price: dec!(14.0),
                    volume: dec!(1.0),
                    commission: dec!(0.0),
                    timestamp: i64::from(i),
                };
                store_clone.process_trade(&a_id, &trade, req_funds).await.map_err(|e| anyhow::anyhow!("Trade DB Error: {}", e))?;
            }
            Ok::<(), anyhow::Error>(())
        }));
    }

    for h in handles {
        h.await.map_err(|e| anyhow::anyhow!("Join error: {}", e))??;
    }
    let elapsed = start.elapsed();
    
    // Final verification
    let snap = store.snapshot(&acct).await.map_err(|e| anyhow::anyhow!(e))?;
    
    // Ensure it didn't take an unreasonable amount of time
    assert!(elapsed.as_millis() < 5000, "50 concurrent txs should be fast");
    
    // Initially 1000. 50 trades of 14$ cost = 700$ spent. Remaining cash must be strictly 300.
    assert_eq!(snap.available_balance, dec!(300.0));
    // No frozen funds left dangling
    assert_eq!(snap.frozen_balance, dec!(0.0));
    // Must own exactly 50 AAPL shares
    assert_eq!(snap.positions.len(), 1);
    assert_eq!(snap.positions[0].symbol, "AAPL");
    assert_eq!(snap.positions[0].volume, dec!(50.0));
    Ok(())
}
