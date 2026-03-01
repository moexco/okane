use okane_core::engine::error::EngineError;
use okane_core::engine::port::{EngineBuilder, EngineFuture, EngineBuildParams};
use okane_core::market::port::Market;
use okane_core::strategy::entity::EngineType;

use std::sync::Arc;

use crate::quickjs::JsEngine;
use crate::wasm::WasmEngine;

/// # Summary
/// `EngineBuilder` 的具体实现。
/// 根据 `EngineType` 选择 `JsEngine` 或 `WasmEngine` 构建执行任务。
///
/// # Invariants
/// - 持有 `Arc<dyn Market>` 用于创建具体引擎实例。
/// - JS 引擎因 QuickJS 的 `!Send` 限制，通过独立线程 + `LocalSet` 执行。
pub struct EngineFactory {
    // 市场数据接口，构造引擎时注入
    market: Arc<dyn Market>,
}

impl EngineFactory {
    /// # Summary
    /// 创建 EngineFactory 实例。
    ///
    /// # Arguments
    /// * `market` - 市场数据接口的具体实现。
    ///
    /// # Returns
    /// * `Self`
    pub fn new(market: Arc<dyn Market>) -> Self {
        Self { market }
    }
}

impl EngineBuilder for EngineFactory {
    /// # Summary
    /// 根据引擎类型构建策略执行 Future。
    ///
    /// # Logic
    /// 1. 根据 engine_type 选择 JsEngine 或 WasmEngine。
    /// 2. 注册所有 SignalHandler。
    /// 3. 对于 JsEngine：因 QuickJS AsyncRuntime 不是 Send，
    ///    使用独立线程 + tokio LocalSet 运行，通过 oneshot 通道桥接结果。
    /// 4. 对于 WasmEngine：直接包装为 Send Future。
    fn build(
        &self,
        params: EngineBuildParams,
    ) -> Result<EngineFuture, EngineError> {
        let market = self.market.clone();

        match params.engine_type {
            EngineType::JavaScript => {
                let js_source = String::from_utf8(params.source).map_err(|e| {
                    EngineError::Plugin(format!("Invalid UTF-8 in JS source: {}", e))
                })?;

                // QuickJS AsyncRuntime 不是 Send，必须在单独的线程上使用 LocalSet 运行
                Ok(Box::pin(async move {
                    let (tx, rx) = tokio::sync::oneshot::channel();

                    std::thread::spawn(move || {
                        let rt_res = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build();

                        let rt = match rt_res {
                            Ok(rt) => rt,
                            Err(e) => {
                                let _ = tx.send(Err(EngineError::Plugin(format!("Failed to build tokio current_thread runtime: {}", e))));
                                return;
                            }
                        };

                        let local = tokio::task::LocalSet::new();
                        local.block_on(&rt, async move {
                            let mut engine = JsEngine::new(market, params.trade_port, params.time_provider);
                            for handler in params.handlers {
                                engine.register_handler(handler);
                            }
                            let result = engine
                                .run_strategy(&params.symbol, &params.account_id, params.timeframe, &js_source)
                                .await;
                            let _ = tx.send(result);
                        });
                    });

                    rx.await.map_err(|_| {
                        EngineError::Plugin("JS engine thread terminated unexpectedly".to_string())
                    })?
                }))
            }
            EngineType::Wasm => {
                // WasmEngine 的 Future 是 Send 的，可以直接包装
                Ok(Box::pin(async move {
                    let mut engine = WasmEngine::new(market, params.trade_port, params.time_provider);
                    for handler in params.handlers {
                        engine.register_handler(handler);
                    }
                    engine
                        .run_strategy(&params.symbol, &params.account_id, params.timeframe, &params.source)
                        .await
                }))
            }
        }
    }
}
