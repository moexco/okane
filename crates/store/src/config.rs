use std::path::PathBuf;
use std::sync::OnceLock;

static ROOT_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 设置存储层的数据根目录。
///
/// # Logic
/// 1. 尝试将指定的路径保存到全局静态变量中。
/// 2. 如果已经设置过，则本次设置无效。
///
/// # Arguments
/// * `path` - 存储数据的根目录路径。
///
/// # Returns
/// * None
pub fn set_root_dir(path: PathBuf) {
    let _ = ROOT_DIR.set(path);
}

/// 获取当前配置的数据根目录。
///
/// # Logic
/// 1. 检查全局静态变量 `ROOT_DIR` 是否已初始化。
/// 2. 若已初始化则返回其克隆，否则返回默认路径 "data"。
///
/// # Arguments
/// * None
///
/// # Returns
/// * 返回配置的根目录路径。
pub(crate) fn get_root_dir() -> PathBuf {
    ROOT_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("data"))
}
