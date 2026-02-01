use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use okane_core::common::{Stock, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, MarketDataProvider};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tokio_stream::wrappers::ReceiverStream;

/// # Summary
/// Yahoo Finance 行情提供者实现。
///
/// # Invariants
/// - 使用 `reqwest` 异步客户端进行通讯。
#[derive(Clone)]
pub struct YahooProvider {
    /// 内部使用的 HTTP 客户端
    client: Client,
}

impl YahooProvider {
    /// # Summary
    /// 创建一个新的 YahooProvider 实例。
    ///
    /// # Logic
    /// 1. 配置 10 秒超时。
    /// 2. 设置伪装浏览器 Header (User-Agent) 以减少被拦截风险。
    /// 3. 初始化 reqwest 客户端。
    ///
    /// # Arguments
    /// * None
    ///
    /// # Returns
    /// 返回初始化后的 YahooProvider。
    pub fn new() -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36".parse().unwrap()
        );

        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .default_headers(headers)
                .build()
                .expect("Failed to build HTTP client"),
        }
    }
}

impl Default for YahooProvider {
    /// # Summary
    /// 提供 YahooProvider 的默认初始化。
    ///
    /// # Logic
    /// 1. 调用 `Self::new()` 进行初始化。
    ///
    /// # Arguments
    /// * None
    ///
    /// # Returns
    /// 返回默认的 YahooProvider 实例。
    fn default() -> Self {
        Self::new()
    }
}

/// # Summary
/// Yahoo API 响应顶层结构。
///
/// # Invariants
/// - 映射自 Yahoo v8 chart 接口。
#[derive(Deserialize, Debug)]
struct YahooResponse {
    chart: YahooChart,
}

/// # Summary
/// Yahoo API 图表数据部分。
#[derive(Deserialize, Debug)]
struct YahooChart {
    result: Option<Vec<YahooResult>>,
    error: Option<YahooError>,
}

/// # Summary
/// Yahoo API 错误详情。
#[derive(Deserialize, Debug)]
struct YahooError {
    description: String,
}

/// # Summary
/// Yahoo API 单个时间序列结果。
#[derive(Deserialize, Debug)]
struct YahooResult {
    timestamp: Vec<i64>,
    indicators: YahooIndicators,
}

/// # Summary
/// Yahoo API 指标容器。
#[derive(Deserialize, Debug)]
struct YahooIndicators {
    quote: Vec<YahooQuote>,
    // 调整后的价格数据
    adjclose: Option<Vec<YahooAdjClose>>,
}

/// # Summary
/// Yahoo API 调整后价格结构。
#[derive(Deserialize, Debug)]
struct YahooAdjClose {
    // 调整后的收盘价列表
    adjclose: Vec<Option<f64>>,
}

/// # Summary
/// Yahoo API 原始报价数据。
#[derive(Deserialize, Debug)]
struct YahooQuote {
    /// 开盘价列表
    open: Vec<Option<f64>>,
    /// 最高价列表
    high: Vec<Option<f64>>,
    /// 最低价列表
    low: Vec<Option<f64>>,
    /// 收盘价列表
    close: Vec<Option<f64>>,
    /// 成交量列表
    volume: Vec<Option<f64>>,
}

#[async_trait]
impl MarketDataProvider for YahooProvider {
    /// # Summary
    /// 从 Yahoo Finance 抓取 K 线历史数据。
    ///
    /// # Logic
    /// 1. 映射 TimeFrame 周期为 Yahoo 识别的 interval。
    /// 2. 构建包含 period1, period2 的 API URL。
    /// 3. 发起异步请求并解析嵌套的 JSON 数据。
    /// 4. 提取 adjclose 并与基础 OHLCV 合并。
    /// 5. 历史数据一律标记为 is_final = true。
    ///
    /// # Arguments
    /// * `stock`: 证券实体。
    /// * `timeframe`: 周期。
    /// * `start`: 开始时间。
    /// * `end`: 结束时间。
    ///
    /// # Returns
    /// 成功返回 K 线列表，失败返回 MarketError。
    async fn fetch_candles(
        &self,
        stock: &Stock,
        timeframe: TimeFrame,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Candle>, MarketError> {
        let symbol = &stock.symbol;
        let interval = match timeframe {
            TimeFrame::Minute1 => "1m",
            TimeFrame::Minute5 => "5m",
            TimeFrame::Hour1 => "60m",
            TimeFrame::Day1 => "1d",
        };

        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{}",
            symbol
        );

        let resp = self
            .client
            .get(&url)
            .query(&[
                ("symbol", symbol.as_str()),
                ("period1", &start.timestamp().to_string()),
                ("period2", &end.timestamp().to_string()),
                ("interval", interval),
            ])
            .send()
            .await
            .map_err(|e| MarketError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(MarketError::Network(format!("HTTP {}", resp.status())));
        }

        let json: YahooResponse = resp
            .json()
            .await
            .map_err(|e| MarketError::Parse(e.to_string()))?;

        if let Some(err) = json.chart.error {
            return Err(MarketError::Unknown(err.description));
        }

        let result = json
            .chart
            .result
            .ok_or(MarketError::NotFound)?
            .pop()
            .ok_or(MarketError::NotFound)?;

        let mut candles = Vec::new();
        let quote = result
            .indicators
            .quote
            .first()
            .ok_or(MarketError::Parse("No quote data".into()))?;

        let adj_close_list = result
            .indicators
            .adjclose
            .as_ref()
            .and_then(|v| v.first())
            .map(|v| &v.adjclose);

        for (i, &ts) in result.timestamp.iter().enumerate() {
            if let (Some(o), Some(h), Some(l), Some(c), Some(v)) = (
                quote.open.get(i).and_then(|x| *x),
                quote.high.get(i).and_then(|x| *x),
                quote.low.get(i).and_then(|x| *x),
                quote.close.get(i).and_then(|x| *x),
                quote.volume.get(i).and_then(|x| *x),
            ) {
                let adj_c = adj_close_list.and_then(|list| list.get(i)).and_then(|x| *x);

                candles.push(Candle {
                    time: Utc.timestamp_opt(ts, 0).unwrap(),
                    open: o,
                    high: h,
                    low: l,
                    close: c,
                    adj_close: adj_c,
                    volume: v,
                    is_final: true, // 历史数据默认为最终态
                });
            }
        }

        Ok(candles)
    }

    /// # Summary
    /// 订阅实时 K 线流（通过定时轮询模拟）。
    ///
    /// # Logic
    /// 1. 创建异步通道 (mpsc)。
    /// 2. 启动后台任务进行定时轮询。
    /// 3. 根据当前时间判断最新的一根 K 线是否已走完周期（is_final）。
    /// 4. 维护最后推送的时间戳及状态，防止重复推送已完结的数据。
    ///
    /// # Arguments
    /// * `stock`: 证券实体。
    /// * `timeframe`: 周期。
    ///
    /// # Returns
    /// 成功返回异步 K 线流 `CandleStream`，失败返回 `MarketError`。
    async fn subscribe_candles(
        &self,
        stock: &Stock,
        timeframe: TimeFrame,
    ) -> Result<CandleStream, MarketError> {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let provider = self.clone();
        let stock_owned = stock.clone();

        let (poll_interval, cycle_secs) = match timeframe {
            TimeFrame::Minute1 => (Duration::from_secs(60), 60),
            TimeFrame::Minute5 => (Duration::from_secs(300), 300),
            TimeFrame::Hour1 => (Duration::from_secs(3600), 3600),
            TimeFrame::Day1 => (Duration::from_secs(86400), 86400),
        };

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(poll_interval);
            let mut last_pushed_ts = None;
            let mut last_pushed_is_final = false;

            loop {
                interval.tick().await;

                let end = Utc::now();
                let start = end - chrono::Duration::seconds(cycle_secs * 2);

                if let Ok(mut candles) = provider
                    .fetch_candles(&stock_owned, timeframe, start, end)
                    .await
                {
                    for candle in candles.iter_mut() {
                        // 如果 K 线时间 + 周期 <= 当前时间，则认为已收盘
                        let is_closed = candle.time + chrono::Duration::seconds(cycle_secs) <= Utc::now();
                        candle.is_final = is_closed;

                        let should_push = match last_pushed_ts {
                            None => true,
                            Some(ts) if candle.time > ts => true,
                            Some(ts) if candle.time == ts && candle.is_final && !last_pushed_is_final => true,
                            _ => false,
                        };

                        if should_push {
                            last_pushed_ts = Some(candle.time);
                            last_pushed_is_final = candle.is_final;
                            if tx.send(candle.clone()).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}
