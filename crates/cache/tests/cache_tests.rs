use okane_cache::mem::MemCache;
use okane_core::cache::port::{Cache, CacheExt};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct TestItem {
    id: u32,
    name: String,
}

#[tokio::test]
async fn test_mem_cache_raw_ops() {
    let cache = MemCache::new();
    let key = "raw_key";
    let value = vec![1, 2, 3, 4];

    // 测试存取
    cache.set_raw(key, value.clone()).await.unwrap();
    let result = cache.get_raw(key).await.unwrap().unwrap();
    assert_eq!(result, value);

    // 测试删除
    cache.del(key).await.unwrap();
    let result = cache.get_raw(key).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_mem_cache_typed_ops() {
    let cache = MemCache::new();
    let key = "typed_key";
    let item = TestItem {
        id: 42,
        name: "Okane".to_string(),
    };

    // 使用 CacheExt 提供的 set 方法
    cache.set(key, &item).await.unwrap();

    // 使用 CacheExt 提供的 get 方法
    let result: TestItem = cache.get(key).await.unwrap().unwrap();
    assert_eq!(result, item);
}
