//! Async bridge for executing futures from synchronous engine callbacks.
//!
//! Since QuickJS/WASM host functions are inherently synchronous,
//! this bridge provides a single reusable OS thread per engine instance
//! to run async code via `Handle::block_on`, avoiding per-call thread creation.

use okane_core::engine::error::EngineError;
use std::future::Future;
use std::sync::mpsc;
use tokio::runtime::Handle;

type BoxedTask = Box<dyn FnOnce(&Handle) + Send>;

/// A reusable sync-async bridge that maintains a single dedicated OS thread
/// for running futures via `Handle::block_on`.
///
/// # Why this exists
/// `rquickjs` and `wasmtime` host callbacks are synchronous, but our
/// trade/market ports are async. We need a blocking bridge, but spawning
/// a new OS thread per call (as `std::thread::scope` does) is wasteful.
///
/// Since both QuickJS and WASM engines are single-threaded, at most one
/// host call is in flight at any time per engine instance. A single
/// bridge thread per engine is sufficient.
///
/// # Cost
/// ~1µs per call (channel send/recv) vs ~50-100µs (thread create/destroy).
pub struct AsyncBridge {
    task_tx: mpsc::SyncSender<BoxedTask>,
}

impl AsyncBridge {
    /// Create a new AsyncBridge. Must be called from within a tokio runtime context.
    pub fn new() -> Result<Self, EngineError> {
        let handle = Handle::current();
        // Rendezvous channel (capacity 0): sender blocks until receiver is ready.
        // This is fine because there's at most one caller at a time.
        let (tx, rx) = mpsc::sync_channel::<BoxedTask>(0);

        std::thread::Builder::new()
            .name("engine-async-bridge".into())
            .spawn(move || {
                for task in rx {
                    task(&handle);
                }
                // Channel closed => engine dropped, thread exits naturally.
            })
            .map_err(|e| {
                EngineError::Plugin(format!("Failed to spawn async bridge thread: {}", e))
            })?;

        Ok(Self { task_tx: tx })
    }

    /// Execute an async future from a synchronous context, blocking until completion.
    ///
    /// The future is sent to the bridge thread and executed via `Handle::block_on`,
    /// ensuring proper tokio runtime context (timers, I/O, sync primitives all work).
    ///
    /// # Errors
    /// Returns EngineError if the bridge thread has disconnected or dropped the result.
    pub fn call<F, R>(&self, future: F) -> Result<R, EngineError>
    where
        F: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        self.task_tx
            .send(Box::new(move |h: &Handle| {
                result_tx
                    .send(h.block_on(future))
                    .map_err(|_| tracing::error!("AsyncBridge: result receiver dropped"))
                    .ok();
            }))
            .map_err(|_| EngineError::Plugin("Bridge thread disconnected".to_string()))?;
        result_rx
            .recv()
            .map_err(|_| EngineError::Plugin("Bridge thread dropped result".to_string()))
    }
}
