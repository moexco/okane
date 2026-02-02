use thiserror::Error;

/// # Summary
/// 引擎域错误枚举。
///
/// # Invariants
/// - 涵盖策略插件、信号处理及底层数据访问的失败场景。
#[derive(Error, Debug)]
pub enum EngineError {
    // 策略插件执行过程中的错误
    #[error("Plugin execution error: {0}")]
    Plugin(String),
    // 信号处理器执行过程中的错误
    #[error("Handler execution error: {0}")]
    Handler(String),
    // 行情数据获取错误
    #[error("Market error: {0}")]
    Market(String),
}
