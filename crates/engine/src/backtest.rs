use chrono::{DateTime, Utc};
use okane_core::common::time::FakeClockProvider;
use okane_core::common::TimeFrame;
use okane_core::market::port::Market;
use okane_core::trade::port::BacktestTradePort;
use std::sync::Arc;
use tracing::{info, warn};

/// # Summary
/// BacktestDriver：回测驱动器，负责接管时间流向，并利用历史数据主动驱动引擎的运行。
///
/// # Invariants
/// - 在每次提取到一根新的 K 线时，必须先将 FakeClock 拨号到 K 线发生时间，
///   再触发与之绑定的策略上下文的 OnCandle 处理与撮合引擎。
pub struct BacktestDriver {
    market: Arc<dyn Market>,
    time_provider: Arc<FakeClockProvider>,
    trade_port: Arc<dyn BacktestTradePort>,
}

impl BacktestDriver {
    /// # Summary
    /// 创建一个新的回测驱动器实例
    ///
    /// # Arguments
    /// * `market`: 市场数据驱动实现，用于拉取回测期间的历史 K 线。
    /// * `trade_port`: 交易端口，此处应为接管了回测撮合能力的 TradeService。
    /// * `time_provider`: 被完全控盘的时钟服务，它的时间完全受此 Driver 支配。
    pub fn new(
        market: Arc<dyn Market>,
        trade_port: Arc<dyn BacktestTradePort>,
        time_provider: Arc<FakeClockProvider>,
    ) -> Self {
        Self {
            market,
            trade_port,
            time_provider,
        }
    }

    /// # Summary
    /// 给定起始时间、周期和总量，执行整个回测序列跑批。
    /// *注意*: 真正的回测实现需要在 JS/WASM engine 获取完单根 K 线后触发 trade_port tick，
    /// 由于目前 Engine 是挂在 websocket 流上的，我们这里先提供基于历史 K 线的事件发射器。
    pub async fn run(
        &self,
        symbol: &str,
        timeframe: TimeFrame,
        start_time: DateTime<Utc>,
        limit: usize,
    ) -> Result<(), String> {
        info!("BacktestDriver starting for [{symbol}] from {start_time}, limit {limit}");

        let stock = self
            .market
            .get_stock(symbol)
            .await
            .map_err(|e| e.to_string())?;

        // 统一预拉取回测所需的所有历史 K 线
        // 计算一个足够宽的时间窗口，考虑周末和停牌缺口 (使用 2x 缓冲)
        let duration = timeframe.duration() * (limit as i32 * 2);
        let end_time = start_time + duration;

        let history = stock
            .fetch_history(timeframe, start_time, end_time)
            .await
            .map(|h| h.into_iter().take(limit).collect::<Vec<_>>())
            .map_err(|e| e.to_string())?;

        if history.is_empty() {
            warn!("Backtest driver got no historical candles. Engine stops.");
            return Ok(());
        }

        // 以历史数据长度推演时间轴并派发给下游引擎
        for candle in history {
            // 步骤 1: 将时间拨动到当前 K 线的时间
            self.time_provider.set_time(candle.time);

            // 步骤 2: 触发订单路由器的 Tick 检查挂单穿越
            if let Err(e) = self.trade_port.tick(symbol, &candle).await {
                warn!("Backtest tick match error: {}", e);
            }
        }

        info!("BacktestDriver finished iteration.");
        Ok(())
    }
}
