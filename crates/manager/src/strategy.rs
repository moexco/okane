use chrono::Utc;
use dashmap::DashMap;
use okane_core::common::TimeFrame;
use okane_core::engine::error::EngineError;
use okane_core::engine::port::{EngineBuilder, EngineBuildParams};
use okane_core::store::error::StoreError;
use okane_core::strategy::entity::{EngineType, StrategyInstance, StrategyStatus};
use okane_core::strategy::port::StrategyStore;
use std::sync::Arc;
use thiserror::Error;
use tokio::task::{AbortHandle, JoinHandle};
use tracing::{error, info};
use uuid::Uuid;

/// # Summary
/// Manager 层的统一错误类型。
#[derive(Error, Debug)]
pub enum ManagerError {
    #[error("Store error: {0}")]
    Store(#[from] StoreError),
    #[error("Engine error: {0}")]
    Engine(#[from] EngineError),
    #[error("Strategy not found: {0}")]
    NotFound(String),
    #[error("Strategy already running: {0}")]
    AlreadyRunning(String),
}

/// # Summary
/// 策略启动请求。
pub struct StartRequest {
    // 目标证券代码
    pub symbol: String,
    // K 线时间周期
    pub timeframe: TimeFrame,
    // 引擎类型
    pub engine_type: EngineType,
    // 策略源码 (JS) 或字节码 (WASM)
    pub source: Vec<u8>,
}

/// # Summary
/// 策略管理器，系统的应用服务层门面 (Facade)。
/// 编译期仅依赖 `okane-core` 中的 Trait 定义，所有具体实现通过构造函数注入。
///
/// # Invariants
/// - `store` 和 `engine_builder` 必须在构造时由外部提供。
/// - 每个运行中的策略对应一个 tokio 协程，通过 `AbortHandle` 管理其生命周期。
pub struct StrategyManager {
    // 策略持久化接口
    store: Arc<dyn StrategyStore>,
    // 引擎构建接口
    engine_builder: Arc<dyn EngineBuilder>,
    // 交易服务通道
    trade_port: Arc<dyn okane_core::trade::port::TradePort>,
    // 运行中的策略协程句柄，Key 为 "{user_id}_{instance_id}"
    running_tasks: DashMap<String, AbortHandle>,
}

impl StrategyManager {
    /// # Summary
    /// 创建 StrategyManager 实例。
    ///
    /// # Arguments
    /// * `store` - 策略持久化接口的具体实现。
    /// * `engine_builder` - 引擎构建接口的具体实现。
    ///
    /// # Returns
    /// * `Arc<Self>` - 可共享的管理器实例。
    pub fn new(
        store: Arc<dyn StrategyStore>,
        engine_builder: Arc<dyn EngineBuilder>,
        trade_port: Arc<dyn okane_core::trade::port::TradePort>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            engine_builder,
            trade_port,
            running_tasks: DashMap::new(),
        })
    }

    /// # Summary
    /// 启动一个新策略。
    ///
    /// # Logic
    /// 1. 生成唯一实例 ID。
    /// 2. 构建 StrategyInstance 聚合根并持久化为 Pending 状态。
    /// 3. 通过 EngineBuilder 构建策略执行 Future。
    /// 4. 更新状态为 Running 并 tokio::spawn 执行。
    /// 5. 记录 AbortHandle 以支持后续停止操作。
    /// 6. 协程结束后自动更新状态为 Stopped 或 Failed。
    ///
    /// # Arguments
    /// * `user_id` - 用户标识符。
    /// * `req` - 策略启动请求。
    ///
    /// # Returns
    /// * `Result<String, ManagerError>` - 成功返回策略实例 ID。
    pub async fn start_strategy(
        self: &Arc<Self>,
        user_id: &str,
        req: StartRequest,
    ) -> Result<String, ManagerError> {
        let instance_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        // 构建聚合根
        let instance = StrategyInstance {
            id: instance_id.clone(),
            symbol: req.symbol.clone(),
            account_id: String::from("SystemDefault_01"), // 默认分配一个本地账号作为兜底
            timeframe: req.timeframe,
            engine_type: req.engine_type.clone(),
            source: req.source.clone(),
            status: StrategyStatus::Pending,
            created_at: now,
            updated_at: now,
        };

        // 持久化
        self.store.save_instance(user_id, &instance).await?;

        let fut = self.engine_builder.build(EngineBuildParams {
            engine_type: req.engine_type,
            symbol: req.symbol,
            account_id: instance.account_id.clone(),
            timeframe: req.timeframe,
            source: req.source,
            handlers: Vec::new(), // TODO: 外部注入 SignalHandler 列表
            trade_port: self.trade_port.clone(),
        })?;

        // 更新状态为 Running
        self.store
            .update_status(user_id, &instance_id, StrategyStatus::Running)
            .await?;

        // 启动协程
        let task_key = format!("{}_{}", user_id, instance_id);
        let store_clone = self.store.clone();
        let user_id_owned = user_id.to_string();
        let id_owned = instance_id.clone();
        let running_tasks = self.running_tasks.clone();
        let task_key_clone = task_key.clone();

        let handle: JoinHandle<()> = tokio::spawn(async move {
            let result = fut.await;

            // 协程结束后更新状态
            let new_status = match &result {
                Ok(()) => {
                    info!("Strategy {} completed normally", id_owned);
                    StrategyStatus::Stopped
                }
                Err(e) => {
                    error!("Strategy {} failed: {}", id_owned, e);
                    StrategyStatus::Failed(e.to_string())
                }
            };

            let _ = store_clone
                .update_status(&user_id_owned, &id_owned, new_status)
                .await;

            // 清理句柄
            running_tasks.remove(&task_key_clone);
        });

        self.running_tasks
            .insert(task_key, handle.abort_handle());

        Ok(instance_id)
    }

    /// # Summary
    /// 停止一个正在运行的策略。
    ///
    /// # Logic
    /// 1. 从 running_tasks 查找并中止对应的协程。
    /// 2. 更新数据库状态为 Stopped。
    ///
    /// # Arguments
    /// * `user_id` - 用户标识符。
    /// * `id` - 策略实例 ID。
    ///
    /// # Returns
    /// * `Result<(), ManagerError>`
    pub async fn stop_strategy(
        &self,
        user_id: &str,
        id: &str,
    ) -> Result<(), ManagerError> {
        let task_key = format!("{}_{}", user_id, id);

        if let Some((_, handle)) = self.running_tasks.remove(&task_key) {
            handle.abort();
        }

        self.store
            .update_status(user_id, id, StrategyStatus::Stopped)
            .await?;

        Ok(())
    }

    /// # Summary
    /// 列出指定用户的所有策略实例。
    ///
    /// # Arguments
    /// * `user_id` - 用户标识符。
    ///
    /// # Returns
    /// * `Result<Vec<StrategyInstance>, ManagerError>`
    pub async fn list_strategies(
        &self,
        user_id: &str,
    ) -> Result<Vec<StrategyInstance>, ManagerError> {
        Ok(self.store.list_instances(user_id).await?)
    }

    /// # Summary
    /// 获取指定用户的特定策略实例。
    ///
    /// # Arguments
    /// * `user_id` - 用户标识符。
    /// * `id` - 策略实例 ID。
    ///
    /// # Returns
    /// * `Result<StrategyInstance, ManagerError>`
    pub async fn get_strategy(
        &self,
        user_id: &str,
        id: &str,
    ) -> Result<StrategyInstance, ManagerError> {
        Ok(self.store.get_instance(user_id, id).await?)
    }
}
