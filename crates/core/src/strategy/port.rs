use crate::store::error::StoreError;
use crate::strategy::entity::{StrategyInstance, StrategyStatus};
use async_trait::async_trait;

/// # Summary
/// 策略实例的持久化接口。
///
/// # Invariants
/// - 所有操作以 `user_id` 为作用域，确保用户间数据隔离。
/// - 实现类必须保证线程安全 (`Send` + `Sync`)。
#[async_trait]
pub trait StrategyStore: Send + Sync {
    /// # Summary
    /// 保存或更新策略聚合根。
    ///
    /// # Arguments
    /// * `user_id` - 用户标识符。
    /// * `instance` - 待保存的聚合根实体。
    ///
    /// # Returns
    /// * `Result<(), StoreError>`
    async fn save_instance(
        &self,
        user_id: &str,
        instance: &StrategyInstance,
    ) -> Result<(), StoreError>;

    /// # Summary
    /// 获取指定用户的特定策略实例。
    ///
    /// # Arguments
    /// * `user_id` - 用户标识符。
    /// * `id` - 策略实例 ID。
    ///
    /// # Returns
    /// * `Result<StrategyInstance, StoreError>` - 找到则返回实体，否则 `StoreError::NotFound`。
    async fn get_instance(&self, user_id: &str, id: &str) -> Result<StrategyInstance, StoreError>;

    /// # Summary
    /// 更新策略运行状态。
    ///
    /// # Arguments
    /// * `user_id` - 用户标识符。
    /// * `id` - 策略实例 ID。
    /// * `status` - 新的状态值。
    ///
    /// # Returns
    /// * `Result<(), StoreError>`
    async fn update_status(
        &self,
        user_id: &str,
        id: &str,
        status: StrategyStatus,
    ) -> Result<(), StoreError>;

    /// # Summary
    /// 列出指定用户所有的策略实例。
    ///
    /// # Arguments
    /// * `user_id` - 用户标识符。
    ///
    /// # Returns
    /// * `Result<Vec<StrategyInstance>, StoreError>`
    async fn list_instances(&self, user_id: &str) -> Result<Vec<StrategyInstance>, StoreError>;

    /// # Summary
    /// 删除指定用户的策略实例。
    ///
    /// # Arguments
    /// * `user_id` - 用户标识符。
    /// * `id` - 策略实例 ID。
    ///
    /// # Returns
    /// * `Result<(), StoreError>`
    async fn delete_instance(&self, user_id: &str, id: &str) -> Result<(), StoreError>;
}

/// # Summary
/// 策略日志的物理持久化与索引接口。
/// 采用“顺序平铺文件存储原始数据 + SQLite 存储偏移量索引”的混合模式。
#[async_trait]
pub trait StrategyLogPort: Send + Sync {
    /// # Summary
    /// 追加一条日志。
    ///
    /// # Logic
    /// 1. 将日志 entry 序列化为 JSONL 格式并追加到物理文件。
    /// 2. 记录该条日志在文件中的起始偏移量。
    /// 3. 将 (strategy_id, timestamp, level, offset) 写入 SQLite 索引表。
    async fn append_log(
        &self,
        user_id: &str,
        entry: &crate::strategy::entity::StrategyLogEntry,
    ) -> Result<(), StoreError>;

    /// # Summary
    /// 分页查询日志。
    ///
    /// # Logic
    /// 1. 从 SQLite 索引表中根据 offset/limit 查找对应的文件偏移量列表。
    /// 2. 根据偏移量从物理文件中 Seek 并读取原始 JSON 数据。
    async fn query_logs(
        &self,
        user_id: &str,
        strategy_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::strategy::entity::StrategyLogEntry>, StoreError>;
}

/// # Summary
/// 供策略运行时调用的日志记录接口。
pub trait StrategyLogger: Send + Sync {
    fn log(&self, level: crate::strategy::entity::LogLevel, message: String);
}
