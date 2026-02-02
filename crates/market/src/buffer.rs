use serde::{Deserialize, Serialize};

/// # Summary
/// 固定容量的滚动环形缓冲区。
///
/// # Invariants
/// - 内存空间在初始化时一次性分配，后续不再扩容。
/// - 始终保持最近 N 根数据。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RollingBuffer<T> {
    // 内部存储容器
    data: Vec<T>,
    // 最大容量
    capacity: usize,
    // 当前插入位置索引
    cursor: usize,
}

impl<T: Clone> RollingBuffer<T> {
    /// # Summary
    /// 创建一个新的滚动缓冲区。
    ///
    /// # Logic
    /// 调用 Vec::with_capacity 预分配指定大小的内存。
    ///
    /// # Arguments
    /// * `capacity`: 固定容量上限。
    ///
    /// # Returns
    /// 初始化后的 RollingBuffer 实例。
    pub fn new(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            capacity,
            cursor: 0,
        }
    }

    /// # Summary
    /// 向缓冲区推送新元素。
    ///
    /// # Logic
    /// 1. 检查当前长度是否小于容量。
    /// 2. 若未满，则直接 push。
    /// 3. 若已满，则根据 cursor 覆盖旧数据，并递增（取模）cursor。
    ///
    /// # Arguments
    /// * `item`: 待插入的元素。
    ///
    /// # Returns
    /// 无。
    pub fn push(&mut self, item: T) {
        if self.data.len() < self.capacity {
            self.data.push(item);
        } else {
            self.data[self.cursor] = item;
            self.cursor = (self.cursor + 1) % self.capacity;
        }
    }

    /// # Summary
    /// 获取缓冲区中的最后一个元素（最新插入的）。
    ///
    /// # Logic
    /// 1. 若缓冲区为空，返回 None。
    /// 2. 若未满，返回 Vec 的最后一个。
    /// 3. 若已满，返回 cursor 前一个位置的元素。
    ///
    /// # Arguments
    /// 无。
    ///
    /// # Returns
    /// 最新元素的克隆选项。
    pub fn last(&self) -> Option<T> {
        if self.data.is_empty() {
            return None;
        }
        if self.data.len() < self.capacity {
            self.data.last().cloned()
        } else {
            let last_idx = if self.cursor == 0 {
                self.capacity - 1
            } else {
                self.cursor - 1
            };
            self.data.get(last_idx).cloned()
        }
    }

    /// # Summary
    /// 获取按时间（插入顺序）排序的完整数据列表。
    ///
    /// # Logic
    /// 1. 若缓冲区未满，直接克隆整个 Vec。
    /// 2. 若已满，通过 cursor 切割并重组两段数据，确保返回的 Vec 是有序的。
    ///
    /// # Arguments
    /// 无。
    ///
    /// # Returns
    /// 包含所有有效元素的有序 Vec 集合。
    pub fn to_vec(&self) -> Vec<T> {
        if self.data.len() < self.capacity {
            self.data.clone()
        } else {
            let mut result = Vec::with_capacity(self.capacity);
            result.extend(self.data[self.cursor..].iter().cloned());
            result.extend(self.data[..self.cursor].iter().cloned());
            result
        }
    }
}
