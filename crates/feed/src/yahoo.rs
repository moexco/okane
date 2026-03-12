use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, TimeZone, Utc};
use futures::{SinkExt, StreamExt};
use okane_core::common::{Stock, TimeFrame};
use okane_core::market::entity::Candle;
use okane_core::market::error::MarketError;
use okane_core::market::port::{CandleStream, MarketDataProvider};
use prost::Message;
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use serde::Deserialize;
use serde_json;
use std::time::Duration;
use tokio;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;

/// # Summary
/// Yahoo Finance 实时价格数据 Protobuf 结构。
///
/// # Invariants
/// - 字段标签与 Yahoo Finance WebSocket 协议一致。
/// - 时间戳通常为毫秒，本系统内部统一处理。
#[derive(Clone, PartialEq, Message)]
pub struct PricingData {
    /// 证券唯一标识符 (Ticker)
    #[prost(string, tag = "1")]
    pub id: String,
    /// 当前最新成交价
    #[prost(float, tag = "2")]
    pub price: f32,
    /// 最后成交时间戳 (毫秒)
    #[prost(int64, tag = "3")]
    pub time: i64,
    /// 货币单位
    #[prost(string, tag = "4")]
    pub currency: String,
    /// 所属交易所
    #[prost(string, tag = "5")]
    pub exchange: String,
    /// 报价类型 (枚举映射)
    #[prost(int32, tag = "6")]
    pub quote_type: i32,
    /// 市场交易时段 (枚举映射)
    #[prost(int32, tag = "7")]
    pub market_hours: i32,
    /// 当日涨跌幅百分比
    #[prost(float, tag = "8")]
    pub change_percent: f32,
    /// 当日累计成交量
    #[prost(int64, tag = "9")]
    pub day_volume: i64,
    /// 当日最高价
    #[prost(float, tag = "10")]
    pub day_high: f32,
    /// 当日最低价
    #[prost(float, tag = "11")]
    pub day_low: f32,
    /// 当日涨跌金额
    #[prost(float, tag = "12")]
    pub change: f32,
    /// 证券简称
    #[prost(string, tag = "13")]
    pub short_name: String,
    /// 最后单笔成交量
    #[prost(int64, tag = "14")]
    pub last_size: i64,
}

/// # Summary
/// Yahoo Finance 行情提供者实现。
///
/// # Invariants
/// - 使用 `reqwest` 异步客户端进行历史数据通讯。
/// - 使用 WebSocket 进行实时行情订阅。
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
    pub fn new() -> Result<Self, MarketError> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
        );

        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .default_headers(headers)
                .build()
                .map_err(|e| MarketError::Network(format!("Failed to build Yahoo client: {}", e)))?,
        })
    }
}

/// # Summary
/// Yahoo API 响应顶层结构。
#[derive(Deserialize, Debug)]
struct YahooResponse {
    /// 图表数据容器
    chart: YahooChart,
}

/// # Summary
/// Yahoo API 图表数据部分。
#[derive(Deserialize, Debug)]
struct YahooChart {
    /// 成功时的结果列表
    result: Option<Vec<YahooResult>>,
    /// 失败时的错误信息
    error: Option<YahooError>,
}

/// # Summary
/// Yahoo API 错误详情。
#[derive(Deserialize, Debug)]
struct YahooError {
    /// 错误描述文本
    description: String,
}

/// # Summary
/// Yahoo API 单个时间序列结果。
#[derive(Deserialize, Debug)]
struct YahooResult {
    /// 时间戳列表
    timestamp: Vec<i64>,
    /// 原始指标数据
    indicators: YahooIndicators,
}

/// # Summary
/// Yahoo API 指标容器。
#[derive(Deserialize, Debug)]
struct YahooIndicators {
    /// 原始报价列表
    quote: Vec<YahooQuote>,
    /// 调整后的价格数据
    adjclose: Option<Vec<YahooAdjClose>>,
}

/// # Summary
/// Yahoo API 调整后价格结构。
#[derive(Deserialize, Debug)]
struct YahooAdjClose {
    /// 调整后的收盘价列表
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
    /// 4. 历史数据一律标记为 is_final = true。
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

        Self::parse_yahoo_response(json)
    }

    /// # Summary
    /// 订阅实时 K 线流（通过 WebSocket）。
    ///
    /// # Logic
    /// 1. 建立到 Yahoo Finance WebSocket 服务端的连接。
    /// 2. 发送订阅 JSON 消息。
    /// 3. 在后台任务中监听消息流，解析 Base64 编码的 Protobuf 数据。
    /// 4. 解析后的 Tick 数据实时转换为 Candle 并通过通道推送到流中。
    ///
    /// # Arguments
    /// * `stock`: 证券实体。
    /// * `_timeframe`: 周期（WebSocket 提供实时 Tick，不直接按周期聚合）。
    ///
    /// # Returns
    /// 成功返回异步 K 线流 `CandleStream`，失败返回 `MarketError`。
    async fn subscribe_candles(
        &self,
        stock: &Stock,
    ) -> Result<CandleStream, MarketError> {
        let (tx, rx) = tokio::sync::mpsc::channel(1000);
        let symbol = stock.symbol.clone();

        tokio::spawn(async move {
            let url = "wss://streamer.finance.yahoo.com/";
            
            // 建立连接
            let (mut ws_stream, _) = match connect_async(url).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to connect to Yahoo WS: {}", e);
                    return;
                }
            };

            // 发送订阅消息
            let sub_msg = serde_json::json!({
                "subscribe": [symbol]
            });

            if ws_stream.send(WsMessage::Text(sub_msg.to_string().into())).await.is_err() {
                return;
            }

            // 监听消息流
            while let Some(msg) = ws_stream.next().await {
                // 使用 let_chains 语法精简结构
                if let Ok(WsMessage::Text(text)) = msg
                    && let Ok(binary) = base64::engine::general_purpose::STANDARD.decode(text.as_bytes())
                    && let Ok(pricing) = PricingData::decode(&binary[..])
                {
                    // 数据转换：Float 转 Decimal，处理精度
                    let price = Decimal::from_f32(pricing.price).unwrap_or_default();
                    let high = Decimal::from_f32(pricing.day_high).unwrap_or(price);
                    let low = Decimal::from_f32(pricing.day_low).unwrap_or(price);
                    // 显式转换 i128 以符合 lossless 规范
                    let volume = Decimal::from_i128(i128::from(pricing.day_volume)).unwrap_or_default();

                    let candle = Candle {
                        // Yahoo 提供的时间戳通常是毫秒级
                        time: Utc.timestamp_opt(pricing.time / 1000, 0).single().unwrap_or_else(Utc::now),
                        open: price,
                        high,
                        low,
                        close: price,
                        adj_close: None,
                        volume,
                        is_final: false, // 实时 Tick 始终为非最终态
                    };

                    // 发送数据到流，若通道关闭则退出
                    if tx.send(candle).await.is_err() {
                        break;
                    }
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    /// # Summary
    /// 在 Yahoo Finance 中模糊搜索证券。
    ///
    /// # Arguments
    /// * `query`: 搜索关键字。
    ///
    /// # Returns
    /// 匹配到的证券元数据列表。
    async fn search_symbols(
        &self,
        query: &str,
    ) -> Result<Vec<okane_core::store::port::StockMetadata>, MarketError> {
        let url = "https://query2.finance.yahoo.com/v1/finance/search";
        
        let resp = self
            .client
            .get(url)
            .query(&[("q", query), ("quotesCount", "10")])
            .send()
            .await
            .map_err(|e| MarketError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(MarketError::Network(format!("HTTP {}", resp.status())));
        }

        #[derive(Deserialize, Debug)]
        struct YahooSearchQuote {
            /// 证券代码
            symbol: String,
            /// 证券全称
            longname: Option<String>,
            /// 证券简称
            shortname: Option<String>,
            /// 交易所标识
            exchange: Option<String>,
            /// 所属行业
            industry: Option<String>,
        }

        #[derive(Deserialize, Debug)]
        struct YahooSearchResponse {
            /// 匹配到的证券报价列表
            quotes: Vec<YahooSearchQuote>,
        }

        let json: YahooSearchResponse = resp
            .json()
            .await
            .map_err(|e| MarketError::Parse(e.to_string()))?;

        let results = json.quotes.into_iter().filter(|q| {
            !q.symbol.is_empty() && (q.longname.is_some() || q.shortname.is_some())
        }).map(|q| {
            let name = q.longname.or(q.shortname).unwrap_or_else(|| "Unknown".to_string());
            okane_core::store::port::StockMetadata {
                symbol: q.symbol,
                name,
                exchange: q.exchange.unwrap_or_else(|| "N/A".to_string()),
                sector: q.industry,
                currency: "USD".to_string(), 
            }
        }).collect();

        Ok(results)
    }
}

impl YahooProvider {
    /// # Summary
    /// 解析 Yahoo Chart API 返回的 JSON 结构。
    ///
    /// # Logic
    /// 1. 检查 API 是否返回错误。
    /// 2. 提取最近的一个结果对象。
    /// 3. 遍历时间戳，关联对应的 OHLCV 数据。
    /// 4. 将浮点数转换为 Decimal 以确保精度安全。
    fn parse_yahoo_response(json: YahooResponse) -> Result<Vec<Candle>, MarketError> {
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

                let open = Decimal::from_f64_retain(o).ok_or_else(|| MarketError::Parse(format!("Invalid open price: {}", o)))?;
                let high = Decimal::from_f64_retain(h).ok_or_else(|| MarketError::Parse(format!("Invalid high price: {}", h)))?;
                let low = Decimal::from_f64_retain(l).ok_or_else(|| MarketError::Parse(format!("Invalid low price: {}", l)))?;
                let close = Decimal::from_f64_retain(c).ok_or_else(|| MarketError::Parse(format!("Invalid close price: {}", c)))?;
                let volume = Decimal::from_f64_retain(v).ok_or_else(|| MarketError::Parse(format!("Invalid volume: {}", v)))?;

                candles.push(Candle {
                    time: Utc.timestamp_opt(ts, 0).single().ok_or_else(|| MarketError::Parse("Invalid timestamp".into()))?,
                    open,
                    high,
                    low,
                    close,
                    adj_close: adj_c.and_then(Decimal::from_f64_retain),
                    volume,
                    is_final: true,
                });
            }
        }

        Ok(candles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[tokio::test]
    async fn test_parse_yahoo_response_success() -> anyhow::Result<()> {
        let json_str = r#"{"chart":{"result":[{"timestamp":[1625097600],"indicators":{"quote":[{"open":[150.0],"high":[155.0],"low":[149.0],"close":[152.0],"volume":[1000000.0]}],"adjclose":[{"adjclose":[151.0]}]}}]}}"#;
        let json: YahooResponse = serde_json::from_str(json_str)?;

        let candles = YahooProvider::parse_yahoo_response(json)?;
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].open, dec!(150.0));
        assert_eq!(candles[0].high, dec!(155.0));
        assert_eq!(candles[0].low, dec!(149.0));
        assert_eq!(candles[0].close, dec!(152.0));
        assert_eq!(candles[0].adj_close, Some(dec!(151.0)));
        assert_eq!(candles[0].volume, dec!(1000000.0));
        let expected_time = Utc.timestamp_opt(1625097600, 0).single().ok_or_else(|| anyhow::anyhow!("Invalid timestamp"))?;
        assert_eq!(candles[0].time, expected_time);
        Ok(())
    }
}
