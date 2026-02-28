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
    async fn get_instance(
        &self,
        user_id: &str,
        id: &str,
    ) -> Result<StrategyInstance, StoreError>;

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
    async fn list_instances(
        &self,
        user_id: &str,
    ) -> Result<Vec<StrategyInstance>, StoreError>;
}
