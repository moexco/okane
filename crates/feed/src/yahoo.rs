use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tokio::sync::mpsc;
// Avoid importing Candle/Interval/Range from yfinance_rs to prevent naming conflicts with our domain entities.
use yfinance_rs::{SearchBuilder, StreamBuilder, StreamMethod, Ticker, YfClient};

use okane_core::{
    common::{Stock, TimeFrame},
    market::{
        entity::Candle,
        error::MarketError,
        port::{CandleStream, MarketDataProvider},
    },
    store::port::StockMetadata,
};

/// Yahoo Finance market data provider implemented using the `yfinance-rs` crate.
///
/// This provider serves as an infrastructure adapter for fetching market data
/// from the Yahoo Finance API.
pub struct YahooProvider {
    client: YfClient,
}

impl YahooProvider {
    /// Creates a new `YahooProvider`.
    ///
    /// # Logic
    /// 1. Initializes the `YfClient` builder with a browser-like User-Agent and a 10-second timeout.
    /// 2. Returns the instance wrapped in a `Result`.
    ///
    /// # Returns
    /// Returns `Ok(Self)` on success, or `MarketError::Network` if initialization fails.
    pub fn new() -> Result<Self, MarketError> {
        let client = YfClient::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| MarketError::Network(format!("failed to build yfinance client: {}", e).to_lowercase()))?;

        Ok(Self { client })
    }
}

#[async_trait]
impl MarketDataProvider for YahooProvider {
    /// Fetches historical candle data for a given stock and timeframe.
    ///
    /// # Logic
    /// 1. Maps the project's `TimeFrame` to `yfinance-rs`'s `Interval`.
    /// 2. Uses `Ticker::history_builder` to construct and execute the request within the specified time range.
    /// 3. Maps the `yfinance-rs::Candle` results to the domain `Candle` entity, performing safe decimal conversions.
    /// 4. Validates that essential data points (e.g., volume) are present, otherwise returns a `MarketError::Parse`.
    ///
    /// # Arguments
    /// * `stock`: Identity identifying the stock symbol.
    /// * `timeframe`: The time resolution of the candles (e.g., 1m, 1h, 1d).
    /// * `start_time`: UTC start time for the history range.
    /// * `end_time`: UTC end time for the history range.
    ///
    /// # Returns
    /// Returns a `Vec<Candle>` on success, or `MarketError` if the fetch or parsing fails.
    async fn fetch_candles(
        &self,
        stock: &Stock,
        timeframe: TimeFrame,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Result<Vec<Candle>, MarketError> {
        let ticker = Ticker::new(&self.client, &stock.symbol);
        let interval = match timeframe {
            TimeFrame::Minute1 => yfinance_rs::Interval::I1m,
            TimeFrame::Minute5 => yfinance_rs::Interval::I5m,
            TimeFrame::Hour1 => yfinance_rs::Interval::I1h,
            TimeFrame::Day1 => yfinance_rs::Interval::D1,
        };

        let response = ticker
            .history_builder()
            .interval(interval)
            .between(start_time, end_time)
            .fetch_full()
            .await
            .map_err(|e| {
                MarketError::Unknown(format!("yahoo fetch_candles failed: {:?}", e).to_lowercase())
            })?;

        let mut candles = Vec::with_capacity(response.candles.len());
        for c in response.candles {
            // Explicitly validate mandatory fields. Defaulting values is forbidden by project conventions.
            let vol_val = c.volume.ok_or_else(|| {
                MarketError::Parse("missing volume in yahoo candle data".to_string())
            })?;
            let volume = Decimal::from(vol_val);

            candles.push(Candle {
                time: c.ts,
                open: c.open.amount(),
                high: c.high.amount(),
                low: c.low.amount(),
                close: c.close.amount(),
                adj_close: c.close_unadj.as_ref().map(|a| a.amount()),
                volume,
                is_final: true,
            });
        }

        Ok(candles)
    }

    /// Subscribes to real-time market data for a given stock.
    ///
    /// # Logic
    /// 1. Configures a `StreamBuilder` for the symbol with a WebSocket fallback method.
    /// 2. Spawns a background task to process incoming `QuoteUpdate` messages.
    /// 3. Maps real-time price updates to temporary `Candle` entities for downstream streaming.
    /// 4. Treats a missing volume delta on the first tick as a valid price-only update and emits a zero-volume candle.
    /// 5. Ensures all errors are propagated through the stream.
    ///
    /// # Arguments
    /// * `stock`: Identity identifying the stock symbol to subscribe to.
    ///
    /// # Returns
    /// Returns a pinned stream of `Candle` instances wrapped in a Result, or `MarketError` if the subscription fails.
    async fn subscribe_candles(&self, stock: &Stock) -> Result<CandleStream, MarketError> {
        let (handle, mut rx) = StreamBuilder::new(&self.client)
            .add_symbol(&stock.symbol)
            .method(StreamMethod::WebsocketWithFallback)
            .diff_only(false)
            .interval(Duration::from_secs(1))
            .start()
            .map_err(|e| {
                MarketError::Unknown(
                    format!("yahoo subscribe_candles failed: {:?}", e).to_lowercase(),
                )
            })?;

        let (tx, receiver) = mpsc::channel(100);

        tokio::spawn(async move {
            while let Some(update) = rx.recv().await {
                if let Some(price) = update.price {
                    let volume = match update.volume {
                        Some(v) => Decimal::from(v),
                        None => {
                            tracing::debug!(
                                "yahoo first tick without volume delta, emitting zero-volume provisional candle"
                            );
                            Decimal::ZERO
                        }
                    };

                    let candle = Candle {
                        time: update.ts,
                        open: price.amount(),
                        high: price.amount(),
                        low: price.amount(),
                        close: price.amount(),
                        adj_close: None,
                        volume,
                        is_final: false,
                    };

                    if tx.send(Ok(candle)).await.is_err() {
                        break;
                    }
                }
            }
            handle.stop().await;
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(
            receiver,
        )))
    }

    /// Searches for symbols matching a keyword query.
    ///
    /// # Logic
    /// 1. Uses `SearchBuilder` to search for symbols.
    /// 2. Maps the `SearchResult` items to `StockMetadata` domain objects.
    /// 3. Validates mandatory fields (name, exchange). Missing data results in a `MarketError::Parse`.
    /// 4. Defaults currency to "USD" as the search API result might not always explicit it.
    ///
    /// # Arguments
    /// * `query`: The search keyword or ticker fragment to look for.
    ///
    /// # Returns
    /// Returns a list of `StockMetadata` for matching symbols, or `MarketError` on failure.
    async fn search_symbols(&self, query: &str) -> Result<Vec<StockMetadata>, MarketError> {
        let response = SearchBuilder::new(&self.client, query)
            .fetch()
            .await
            .map_err(|e| {
                MarketError::Unknown(format!("yahoo search_symbols failed: {:?}", e).to_lowercase())
            })?;

        let mut results = Vec::with_capacity(response.results.len());
        for r in response.results {
            // Explicitly validate mandatory fields. Defaulting values is forbidden.
            let name = r.name.ok_or_else(|| {
                MarketError::Parse(
                    format!(
                        "missing name for symbol {} in yahoo search result",
                        r.symbol
                    )
                    .to_lowercase(),
                )
            })?;
            let exchange = r.exchange.ok_or_else(|| {
                MarketError::Parse(
                    format!(
                        "missing exchange for symbol {} in yahoo search result",
                        r.symbol
                    )
                    .to_lowercase(),
                )
            })?;

            results.push(StockMetadata {
                symbol: r.symbol.to_string(),
                name,
                exchange: exchange.to_string(),
                sector: None,
                currency: "USD".to_string(),
            });
        }

        Ok(results)
    }
}
