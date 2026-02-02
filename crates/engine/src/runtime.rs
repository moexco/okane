use extism::{Manifest, Plugin, Wasm};
use futures::StreamExt;
use okane_core::common::TimeFrame;
use okane_core::engine::entity::Signal;
use okane_core::engine::error::EngineError;
use okane_core::engine::port::SignalHandler;
use okane_core::market::port::Market;
use std::sync::Arc;
use tracing::{error, info};

/// # Summary
/// 基于 WASM 的策略执行引擎。
///
/// # Invariants
/// - 负责加载策略脚本并绑定到特定个股的数据流。
/// - 执行过程中一旦报错立即停止。
/// - 信号处理通过已注册的钩子 (SignalHandler) 分发。
pub struct WasmEngine {
    // 市场数据访问端口
    market: Arc<dyn Market>,
    // 已注册的信号处理器
    handlers: Vec<Box<dyn SignalHandler>>,
}

impl WasmEngine {
    /// # Summary
    /// 创建引擎实例。
    ///
    /// # Logic
    /// 1. 初始化包含市场驱动和空处理器列表的 WasmEngine。
    ///
    /// # Arguments
    /// * `market`: 市场数据驱动接口。
    ///
    /// # Returns
    /// * `Self` - 初始化后的引擎实例。
    pub fn new(market: Arc<dyn Market>) -> Self {
        Self {
            market,
            handlers: Vec::new(),
        }
    }

    /// # Summary
    /// 注册信号处理器。
    ///
    /// # Logic
    /// 1. 将传入的处理器追加到内部列表中。
    ///
    /// # Arguments
    /// * `handler`: 信号处理钩子实现。
    ///
    /// # Returns
    /// * None
    pub fn register_handler(&mut self, handler: Box<dyn SignalHandler>) {
        self.handlers.push(handler);
    }

    /// # Summary
    /// 运行特定个股的策略。
    ///
    /// # Logic
    /// 1. 初始化 Extism 插件实例。
    /// 2. 获取股票聚合根并订阅指定时间周期的 K 线流。
    /// 3. 在循环中接收 K 线，序列化为 JSON 后传给 WASM 的 `on_candle` 函数。
    /// 4. 若产生信号，则通过 `dispatch_signal` 进行分发。
    /// 5. 任何执行错误将导致循环终止并返回错误。
    ///
    /// # Arguments
    /// * `symbol`: 证券代码。
    /// * `timeframe`: 时间周期。
    /// * `wasm_bytes`: 策略插件的二进制字节。
    ///
    /// # Returns
    /// * `Result<(), EngineError>` - 运行成功或报错。
    pub async fn run_strategy(
        &self,
        symbol: &str,
        timeframe: TimeFrame,
        wasm_bytes: &[u8],
    ) -> Result<(), EngineError> {
        info!(
            "Starting strategy for {} with timeframe {:?}",
            symbol, timeframe
        );

        // 初始化 Extism 插件
        let manifest = Manifest::new([Wasm::data(wasm_bytes.to_vec())]);
        let mut plugin =
            Plugin::new(&manifest, [], true).map_err(|e| EngineError::Plugin(e.to_string()))?;

        // 获取股票聚合根
        let stock = self
            .market
            .get_stock(symbol)
            .await
            .map_err(|e| EngineError::Market(e.to_string()))?;

        let mut stream = stock.subscribe(timeframe);

        // 核心执行循环
        while let Some(candle) = stream.next().await {
            // 序列化输入数据
            let input =
                serde_json::to_string(&candle).map_err(|e| EngineError::Plugin(e.to_string()))?;

            // 调用插件方法
            match plugin.call::<&str, &str>("on_candle", &input) {
                Ok(output) => {
                    // 解析输出信号 (预期返回 Option<Signal> 的 JSON)
                    if let Ok(Some(signal)) = serde_json::from_str::<Option<Signal>>(output) {
                        self.dispatch_signal(signal).await?;
                    }
                }
                Err(e) => {
                    error!("Strategy execution failed for {}: {}", symbol, e);
                    return Err(EngineError::Plugin(e.to_string()));
                }
            }
        }

        Ok(())
    }

    /// # Summary
    /// 分发信号到所有匹配的处理器。
    ///
    /// # Logic
    /// 1. 遍历所有已注册的处理器。
    /// 2. 若匹配则调用其 handle 方法。
    ///
    /// # Arguments
    /// * `signal`: 策略产生的信号。
    ///
    /// # Returns
    /// * `Result<(), EngineError>` - 分发结果。
    async fn dispatch_signal(&self, signal: Signal) -> Result<(), EngineError> {
        for handler in &self.handlers {
            if handler.matches(&signal) {
                handler.handle(signal.clone()).await?;
            }
        }
        Ok(())
    }
}
