pub mod mock_trade;
use okane_core::common::TimeFrame;
use okane_core::engine::error::EngineError;
use okane_core::engine::port::{EngineBuilder, EngineFuture, EngineBuildParams};
use okane_core::strategy::entity::{EngineType, StrategyStatus};
use okane_manager::strategy::{StartRequest, StrategyManager};
use okane_store::strategy::SqliteStrategyStore;

use std::sync::Arc;
use tempfile::tempdir;
use tokio::time::{sleep, Duration};

struct MockEngineBuilder;

impl EngineBuilder for MockEngineBuilder {
    fn build(
        &self,
        _params: EngineBuildParams,
    ) -> Result<EngineFuture, EngineError> {
        Ok(Box::pin(async {
            // 模拟策略运行一段时间
            sleep(Duration::from_millis(100)).await;
            Ok(())
        }))
    }
}

#[tokio::test]
async fn test_strategy_lifecycle() {
    let tmp_dir = tempdir().unwrap();
    okane_store::config::set_root_dir(tmp_dir.path().to_path_buf());

    let store = Arc::new(SqliteStrategyStore::new().unwrap());
    let engine_builder = Arc::new(MockEngineBuilder);
    let trade_port = Arc::new(mock_trade::MockTradePort);
    let manager = StrategyManager::new(store, engine_builder, trade_port);

    let user_id = "test_user";
    let req = StartRequest {
        symbol: "AAPL".to_string(),
        timeframe: TimeFrame::Minute1,
        engine_type: EngineType::JavaScript,
        source: b"console.log('hello')".to_vec(),
    };

    // 1. 启动策略
    let id = manager.start_strategy(user_id, req).await.unwrap();
    
    // 2. 检查状态是否为 Running (由于是异步启动，可能瞬间完成也可能由于 sleep 还在 Running)
    let instance = manager.get_strategy(user_id, &id).await.unwrap();
    // 由于 start_strategy 内部是先 update_status 再 spawn，
    // 这里很大几率能读到 Running。
    assert!(instance.status == StrategyStatus::Running || instance.status == StrategyStatus::Pending);

    // 3. 等待策略自然结束 (Mock 运行 100ms)
    sleep(Duration::from_millis(200)).await;
    
    let instance = manager.get_strategy(user_id, &id).await.unwrap();
    assert_eq!(instance.status, StrategyStatus::Stopped);

    // 4. 下发一个不停止的策略
    struct InfiniteEngineBuilder;
    impl EngineBuilder for InfiniteEngineBuilder {
        fn build(&self, _params: EngineBuildParams) -> Result<EngineFuture, EngineError> {
            Ok(Box::pin(async {
                loop { sleep(Duration::from_secs(1)).await; }
            }))
        }
    }
    
    let manager = StrategyManager::new(Arc::new(SqliteStrategyStore::new().unwrap()), Arc::new(InfiniteEngineBuilder), Arc::new(mock_trade::MockTradePort));
    let req = StartRequest {
        symbol: "AAPL".to_string(),
        timeframe: TimeFrame::Minute1,
        engine_type: EngineType::JavaScript,
        source: b"loop".to_vec(),
    };
    let id = manager.start_strategy(user_id, req).await.unwrap();
    
    // 5. 立即停止
    manager.stop_strategy(user_id, &id).await.unwrap();
    
    let instance = manager.get_strategy(user_id, &id).await.unwrap();
    assert_eq!(instance.status, StrategyStatus::Stopped);
}
