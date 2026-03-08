use okane_core::test_utils::{SpyTradePort, MockAlgoOrderPort, MockIndicatorService};
use okane_core::common::TimeFrame;
use okane_core::engine::error::EngineError;
use okane_core::engine::port::{EngineBuilder, EngineFuture, EngineBuildParams};
use okane_core::notify::port::NotifierFactory;
use okane_core::strategy::entity::{EngineType, StrategyStatus};
use okane_manager::strategy::{StartRequest, StrategyManager};
use okane_store::strategy::SqliteStrategyStore;

use std::sync::Arc;
use tempfile::tempdir;
use tokio::time::{sleep, Duration};

/// 测试用空通知工厂, 始终返回 None
struct NoopNotifierFactory;

#[async_trait::async_trait]
impl NotifierFactory for NoopNotifierFactory {
    async fn create_for_user(&self, _user_id: &str) -> Result<Option<Arc<dyn okane_core::notify::port::Notifier>>, okane_core::notify::error::NotifyError> {
        Ok(None)
    }
}

struct MockEngineBuilder;

impl EngineBuilder for MockEngineBuilder {
    fn build(
        &self,
        _params: EngineBuildParams,
    ) -> Result<EngineFuture, EngineError> {
        Ok(Box::pin(async {
            // 模拟策略运行一段时间
            sleep(Duration::from_millis(50)).await;
            Ok(())
        }))
    }
}

#[tokio::test]
async fn test_strategy_lifecycle() -> anyhow::Result<()> {
    let tmp_dir = tempdir().map_err(|e| anyhow::anyhow!("Failed to create temp dir: {}", e))?;
    okane_store::config::set_root_dir(tmp_dir.path().to_path_buf());

    let store = Arc::new(SqliteStrategyStore::new().map_err(|e| anyhow::anyhow!("Failed to create store: {}", e))?);
    let engine_builder = Arc::new(MockEngineBuilder);
    let trade_port = Arc::new(SpyTradePort::new());
    let manager = StrategyManager::new(
        store,
        engine_builder as Arc<dyn okane_core::engine::port::EngineBuilder>,
        trade_port,
        Arc::new(MockAlgoOrderPort),
        Arc::new(MockIndicatorService),
        Arc::new(okane_core::common::time::RealTimeProvider),
        Arc::new(NoopNotifierFactory),
    );

    let user_id = "test_user";
    let req = StartRequest {
        symbol: "AAPL".to_string(),
        account_id: "SystemDefault_01".to_string(),
        timeframe: TimeFrame::Minute1,
        engine_type: EngineType::JavaScript,
        source: b"console.log('hello')".to_vec(),
    };

    // 1. 启动策略
    let id = manager.start_strategy(user_id, req).await.map_err(|e| anyhow::anyhow!("Start failed: {:?}", e))?;
    
    // 2. 检查状态
    let instance = manager.get_strategy(user_id, &id).await.map_err(|e| anyhow::anyhow!("Get failed: {:?}", e))?;
    assert!(instance.status == StrategyStatus::Running || instance.status == StrategyStatus::Pending || instance.status == StrategyStatus::Stopped);

    // 3. 轮询等待策略停止 (Mock 运行 50ms)
    let start = std::time::Instant::now();
    let mut stopped = false;
    while start.elapsed() < Duration::from_secs(2) {
        let instance = manager.get_strategy(user_id, &id).await.map_err(|e| anyhow::anyhow!("Poll failed: {:?}", e))?;
        if instance.status == StrategyStatus::Stopped {
            stopped = true;
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }
    assert!(stopped, "Strategy should have stopped within 2s");

    // 4. 下发一个不停止的策略
    struct InfiniteEngineBuilder;
    impl EngineBuilder for InfiniteEngineBuilder {
        fn build(&self, _params: EngineBuildParams) -> Result<EngineFuture, EngineError> {
            Ok(Box::pin(async {
                loop { sleep(Duration::from_millis(100)).await; }
            }))
        }
    }
    
    let store_inf = Arc::new(SqliteStrategyStore::new().map_err(|e| anyhow::anyhow!("Failed to create inf store: {}", e))?);
    let manager_inf = StrategyManager::new(
        store_inf,
        Arc::new(InfiniteEngineBuilder) as Arc<dyn okane_core::engine::port::EngineBuilder>,
        Arc::new(SpyTradePort::new()),
        Arc::new(MockAlgoOrderPort),
        Arc::new(MockIndicatorService),
        Arc::new(okane_core::common::time::RealTimeProvider),
        Arc::new(NoopNotifierFactory),
    );
    let req_inf = StartRequest {
        symbol: "AAPL".to_string(),
        account_id: "SystemDefault_01".to_string(),
        timeframe: TimeFrame::Minute1,
        engine_type: EngineType::JavaScript,
        source: b"loop".to_vec(),
    };
    let id_inf = manager_inf.start_strategy(user_id, req_inf).await.map_err(|e| anyhow::anyhow!("Start inf failed: {:?}", e))?;
    
    // 5. 立即停止
    manager_inf.stop_strategy(user_id, &id_inf).await.map_err(|e| anyhow::anyhow!("Stop inf failed: {:?}", e))?;
    
    let instance_inf = manager_inf.get_strategy(user_id, &id_inf).await.map_err(|e| anyhow::anyhow!("Get inf failed: {:?}", e))?;
    assert_eq!(instance_inf.status, StrategyStatus::Stopped);
    Ok(())
}
