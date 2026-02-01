use crate::cache::error::CacheError;
use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};

/// # Summary
/// 业务无关的异步 KV 存储接口 (Port)。
///
/// # Invariants
/// - 处理原始字节，确保 Trait 是对象安全的 (Object Safe)。
/// - 数据生命周期与管理逻辑由上游业务层实现。
#[async_trait]
pub trait Cache: Send + Sync {
    /// # Summary
    /// 设置原始字节数据。
    ///
    /// # Logic
    /// 1. 将数据以原子方式写入内存或持久化介质。
    ///
    /// # Arguments
    /// * `key`: 唯一键。
    /// * `value`: 原始字节数组。
    ///
    /// # Returns
    /// 成功返回 Ok，失败返回 `CacheError`。
    async fn set_raw(&self, key: &str, value: Vec<u8>) -> Result<(), CacheError>;

    /// # Summary
    /// 获取原始字节数据。
    ///
    /// # Logic
    /// 1. 根据键检索存储内容。
    ///
    /// # Arguments
    /// * `key`: 唯一键。
    ///
    /// # Returns
    /// 存在则返回 `Some(Vec<u8>)`，否则返回 `None`。
    async fn get_raw(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError>;

    /// # Summary
    /// 删除指定键。
    ///
    /// # Logic
    /// 1. 移除键值对并释放空间。
    ///
    /// # Arguments
    /// * `key`: 唯一键。
    ///
    /// # Returns
    /// 成功返回 Ok。
    async fn del(&self, key: &str) -> Result<(), CacheError>;
}

/// # Summary
/// 缓存泛型扩展接口，提供便捷的序列化支持。
///
/// # Invariants
/// - 自动为所有实现 `Cache` 的类型提供支持。
#[async_trait]
pub trait CacheExt: Cache {
    /// # Summary
    /// 存入强类型对象。
    ///
    /// # Logic
    /// 1. 使用 JSON 序列化对象。
    /// 2. 调用底层 `set_raw` 写入。
    ///
    /// # Arguments
    /// * `key`: 唯一键。
    /// * `value`: 实现了 Serialize 的对象引用。
    ///
    /// # Returns
    /// 操作结果。
    async fn set<T: Serialize + Send + Sync>(
        &self,
        key: &str,
        value: &T,
    ) -> Result<(), CacheError> {
        let bytes = serde_json::to_vec(value).map_err(|e| CacheError::Serialize(e.to_string()))?;
        self.set_raw(key, bytes).await
    }

    /// # Summary
    /// 取出强类型对象。
    ///
    /// # Logic
    /// 1. 调用底层 `get_raw` 获取字节。
    /// 2. 使用 JSON 反序列化为目标类型。
    ///
    /// # Arguments
    /// * `key`: 唯一键。
    ///
    /// # Returns
    /// 反序列化后的对象或 None。
    async fn get<T: DeserializeOwned + Send>(&self, key: &str) -> Result<Option<T>, CacheError> {
        match self.get_raw(key).await? {
            Some(bytes) => {
                let val = serde_json::from_slice(&bytes)
                    .map_err(|e| CacheError::Deserialize(e.to_string()))?;
                Ok(Some(val))
            }
            None => Ok(None),
        }
    }
}

impl<T: Cache + ?Sized> CacheExt for T {}
