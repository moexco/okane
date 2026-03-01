use chrono::{DateTime, Utc};
use std::sync::RwLock;

/// # Summary
/// 时间供给器接口，用于劫持和隔离物理系统时钟。
/// 所有的引擎沙盒与历史回放必须通过此接口获取当前挂载时间。
pub trait TimeProvider: Send + Sync {
    /// 获取当前挂载的时间
    fn now(&self) -> DateTime<Utc>;
}

/// # Summary
/// 针对实盘和普通运行的真实时钟，直接返回操作系统当前时间。
pub struct RealTimeProvider;

impl TimeProvider for RealTimeProvider {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// # Summary
/// 回测专用虚拟时钟，允许系统主动拨快或回退时间。
///
/// # Invariants
/// - 并发安全：内部利用 `RwLock` 提供给多线程安全修改和读取时间的权限。
pub struct FakeClockProvider {
    current_time: RwLock<DateTime<Utc>>,
}

impl FakeClockProvider {
    /// 使用指定的初始时间创建虚拟时钟
    pub fn new(initial_time: DateTime<Utc>) -> Self {
        Self {
            current_time: RwLock::new(initial_time),
        }
    }

    /// 强制修改时钟的当前时间
    pub fn set_time(&self, new_time: DateTime<Utc>) {
        let mut time = self.current_time.write().unwrap();
        *time = new_time;
    }
}

impl TimeProvider for FakeClockProvider {
    fn now(&self) -> DateTime<Utc> {
        *self.current_time.read().unwrap()
    }
}
