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
use tracing;

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
    /// 最后成交时间戳 (通常为毫秒)
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
    /// 到期日 (用于期权/期货)
    #[prost(int64, tag = "14")]
    pub expire_date: i64,
    /// 开盘价
    #[prost(float, tag = "15")]
    pub open_price: f32,
    /// 昨收价
    #[prost(float, tag = "16")]
    pub previous_close: f32,
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
                .map_err(|e| MarketError::Network(format!("failed to build yahoo client: {}", e)))?,
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
            .map_err(|e| MarketError::Network(format!("failed to fetch yahoo history: {}", e)))?;

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
            let mut backoff = Duration::from_secs(1);
            let max_backoff = Duration::from_secs(60);

            loop {
                tracing::info!("connecting to yahoo ws for {}...", symbol);
                
                let conn_res = connect_async(url).await;
                let (mut ws_stream, _) = match conn_res {
                    Ok(s) => {
                        backoff = Duration::from_secs(1); // 连接成功，重置退避
                        s
                    },
                    Err(e) => {
                        let err_msg = format!("failed to connect to yahoo ws: {}", e);
                        tracing::error!("{}", err_msg);
                        if tx.send(Err(MarketError::Network(err_msg))).await.is_err() {
                            break;
                        }
                        tokio::time::sleep(backoff).await;
                        backoff = std::cmp::min(backoff * 2, max_backoff);
                        continue;
                    }
                };

                // 发送订阅消息
                let sub_msg = serde_json::json!({ "subscribe": [symbol] });
                if let Err(e) = ws_stream.send(WsMessage::Text(sub_msg.to_string().into())).await {
                    let err_msg = format!("failed to send subscription: {}", e);
                    tracing::error!("{}", err_msg);
                    if tx.send(Err(MarketError::Network(err_msg))).await.is_err() {
                        break;
                    }
                    tokio::time::sleep(backoff).await;
                    continue;
                }

                // 监听消息流
                while let Some(msg) = ws_stream.next().await {
                    match msg {
                        Ok(WsMessage::Text(text)) => {
                            let binary = match base64::engine::general_purpose::STANDARD.decode(text.as_bytes()) {
                                Ok(b) => b,
                                Err(e) => {
                                    tracing::warn!("failed to decode base64: {}", e);
                                    continue;
                                }
                            };

                            let pricing = match PricingData::decode(&binary[..]) {
                                Ok(p) => p,
                                Err(e) => {
                                    tracing::warn!("failed to decode protobuf: {}", e);
                                    continue;
                                }
                            };

                            // 数据转换
                            let Some(price) = Decimal::from_f32(pricing.price) else {
                                tracing::warn!("yahoo ws: invalid price: {}", pricing.price);
                                continue;
                            };

                            // 严禁掩盖错误：若高低价缺失或为 0 (Protobuf 默认值)，判定数据不完整，跳过。
                            let high = Decimal::from_f32(pricing.day_high).filter(|d| !d.is_zero());
                            let low = Decimal::from_f32(pricing.day_low).filter(|d| !d.is_zero());
                            
                            let (high, low) = if let (Some(h), Some(l)) = (high, low) {
                                (h, l)
                            } else {
                                tracing::warn!("yahoo ws: incomplete ohlc data (high: {:?}, low: {:?}) for {}", high, low, symbol);
                                continue;
                            };

                            // 显式转换 i128
                            let Some(volume) = Decimal::from_i128(i128::from(pricing.day_volume)) else {
                                tracing::warn!("yahoo ws: invalid volume: {}", pricing.day_volume);
                                continue;
                            };

                            // 时间戳修正逻辑：
                            // Yahoo 的 time 字段 (Tag 3) 有时会返回过期或异常值。
                            // 优先使用 time，但如果 time 高于当前时间太久或为 0，则考虑 skip。
                            let raw_ts = pricing.time;
                            // 校验是否为毫秒（Unixtime 10^12 级别为 ms, 10^9 为 s）
                            let ts_sec = if raw_ts > 10_000_000_000 { raw_ts / 1000 } else { raw_ts };
                            
                            let time = match Utc.timestamp_opt(ts_sec, 0).single() {
                                Some(t) => {
                                    // 2028 Bug 检查：如果时间在未来 1 年以上，判定为异常数据
                                    if t.timestamp() > Utc::now().timestamp() + 31536000 {
                                        tracing::error!("yahoo ws: suspicious timestamp (2028 bug?): {} for {}", t, symbol);
                                        continue;
                                    }
                                    t
                                },
                                None => {
                                    tracing::error!("yahoo ws: invalid timestamp: {}", raw_ts);
                                    continue;
                                }
                            };

                            let candle = Candle {
                                time,
                                open: price,
                                high,
                                low,
                                close: price,
                                adj_close: None,
                                volume,
                                is_final: false,
                            };

                            if tx.send(Ok(candle)).await.is_err() {
                                return;
                            }
                        }
                        Ok(_) => continue,
                        Err(e) => {
                            tracing::error!("ws connection error: {}", e);
                            break; // 退出内部循环以触发外部重连
                        }
                    }
                }
                
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, max_backoff);
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    /// # Summary
    /// # Logic
    /// 1. 构建 Yahoo Finance 搜索 API URL。
    /// 2. 发起 GET 请求并解析返回的证券列表。
    /// 3. 过滤无效数据并保留最近的 10 条匹配项。
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
            return Err(MarketError::Network(format!("http {}", resp.status())));
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
            /// 货币
            currency: Option<String>,
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
            // 严禁使用默认值：必须确保核心元数据完整
            !q.symbol.is_empty() 
            && (q.longname.is_some() || q.shortname.is_some())
            && q.exchange.is_some()
            && q.currency.is_some()
        }).filter_map(|q| {
            let name = q.longname.or(q.shortname)?;
            let exchange = q.exchange?;
            let currency = q.currency?;
            
            Some(okane_core::store::port::StockMetadata {
                symbol: q.symbol,
                name,
                exchange,
                sector: q.industry,
                currency, 
            })
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
    ///
    /// # Arguments
    /// * `json`: Yahoo API 返回的 JSON 响应体。
    ///
    /// # Returns
    /// 成功返回 K 线列表，失败返回 `MarketError`。
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
            .ok_or(MarketError::Parse("no quote data".into()))?;

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

                let open = Decimal::from_f64_retain(o).ok_or_else(|| MarketError::Parse(format!("invalid open price: {}", o)))?;
                let high = Decimal::from_f64_retain(h).ok_or_else(|| MarketError::Parse(format!("invalid high price: {}", h)))?;
                let low = Decimal::from_f64_retain(l).ok_or_else(|| MarketError::Parse(format!("invalid low price: {}", l)))?;
                let close = Decimal::from_f64_retain(c).ok_or_else(|| MarketError::Parse(format!("invalid close price: {}", c)))?;
                let volume = Decimal::from_f64_retain(v).ok_or_else(|| MarketError::Parse(format!("invalid volume: {}", v)))?;

                candles.push(Candle {
                    time: Utc.timestamp_opt(ts, 0).single().ok_or_else(|| MarketError::Parse("invalid timestamp".into()))?,
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
