use async_trait::async_trait;
use dashmap::DashMap;
use okane_core::cache::error::CacheError;
use okane_core::cache::port::Cache;

/// # Summary
/// 基于 DashMap 的内存缓存实现。
///
/// # Invariants
/// - 所有操作均通过并发哈希表 `DashMap` 执行，保证多线程安全。
/// - 不提供自动过期或容量限制，数据由业务逻辑管理。
pub struct MemCache {
    // 线程安全的 KV 存储容器
    storage: DashMap<String, Vec<u8>>,
}

impl MemCache {
    /// # Summary
    /// 创建一个新的 MemCache 实例。
    ///
    /// # Logic
    /// 初始化底层的 DashMap 存储引擎。
    ///
    /// # Arguments
    /// * None
    ///
    /// # Returns
    /// * `Self` - 初始化的缓存实例。
    pub fn new() -> Self {
        Self {
            storage: DashMap::new(),
        }
    }
}

impl Default for MemCache {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Cache for MemCache {
    /// # Summary
    /// 设置原始字节数据。
    ///
    /// # Logic
    /// 将 Key 转换为 String 后与 Value 一并插入哈希表。若存在同名 Key 则覆盖。
    ///
    /// # Arguments
    /// * `key`: 唯一索引。
    /// * `value`: 待存入的字节序列。
    ///
    /// # Returns
    /// * `Result<(), CacheError>` - 始终返回 Ok，除非内存分配失败。
    async fn set_raw(&self, key: &str, value: Vec<u8>) -> Result<(), CacheError> {
        self.storage.insert(key.to_string(), value);
        Ok(())
    }

    /// # Summary
    /// 获取原始字节数据。
    ///
    /// # Logic
    /// 从哈希表中检索 Key 对应的引用，并将其克隆为独立的所有权对象返回。
    ///
    /// # Arguments
    /// * `key`: 唯一索引。
    ///
    /// # Returns
    /// * `Result<Option<Vec<u8>>, CacheError>` - 存在则返回克隆的数据，否则返回 None。
    async fn get_raw(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        Ok(self.storage.get(key).map(|v| v.value().clone()))
    }

    /// # Summary
    /// 删除指定键。
    ///
    /// # Logic
    /// 从哈希表中执行原子移除操作。
    ///
    /// # Arguments
    /// * `key`: 待删除的唯一索引。
    ///
    /// # Returns
    /// * `Result<(), CacheError>` - 无论键是否存在均返回 Ok。
    async fn del(&self, key: &str) -> Result<(), CacheError> {
        self.storage.remove(key);
        Ok(())
    }
}
