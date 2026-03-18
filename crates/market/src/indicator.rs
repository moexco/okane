use async_trait::async_trait;
use chrono::Utc;
use okane_core::common::TimeFrame;
use okane_core::market::error::MarketError;
use okane_core::market::indicator::IndicatorService;
use okane_core::market::port::Market;
use rust_decimal::Decimal;
use rust_decimal::prelude::*;
use std::sync::Arc;

pub struct MarketIndicatorService {
    market: Arc<dyn Market>,
}

/// 指标计算预热倍率。
/// 对于 EMA、RSI 等通过递归定义的指标，初始值（通常是 SMA）的影响需要一定量的数据才能消退。
/// 根据传统的工程实践，2-3 倍的周期长度通常足以保证数值收敛到稳定的精度。
const CONVERGENCE_WARMUP_FACTOR: u32 = 3;

impl MarketIndicatorService {
    /// # Logic
    /// Construct a market indicator service backed by a market port.
    ///
    /// # Arguments
    /// * `market` - Market port used to load historical candle data.
    ///
    /// # Returns
    /// * `Self` - A new indicator service instance.
    pub fn new(market: Arc<dyn Market>) -> Self {
        Self { market }
    }

    async fn get_closing_prices(
        &self,
        symbol: &str,
        timeframe: TimeFrame,
        period: u32,
    ) -> Result<Vec<Decimal>, MarketError> {
        let stock = self.market.get_stock(symbol).await?;
        let end = Utc::now();

        // 为了使 EMA/RSI 等递归指标收敛，需要获取比 period 更多的数据作为预热
        let total_limit = period.saturating_mul(CONVERGENCE_WARMUP_FACTOR);
        let limit_i32 = i32::try_from(total_limit)
            .map_err(|_| MarketError::Parse("indicator warmup limit too large".into()))?;
        let duration = timeframe.duration() * limit_i32;
        let start = end - duration;

        let candles = stock.fetch_history(timeframe, start, end).await?;
        let prices: Vec<Decimal> = candles.into_iter().map(|c| c.close).collect();

        let period_usize =
            usize::try_from(period).map_err(|_| MarketError::Parse("period too large".into()))?;

        if prices.len() < period_usize {
            return Err(MarketError::Parse(format!(
                "insufficient data: required {}, actual {}",
                period,
                prices.len()
            )));
        }

        Ok(prices)
    }
}

#[async_trait]
impl IndicatorService for MarketIndicatorService {
    async fn sma(
        &self,
        symbol: &str,
        timeframe: TimeFrame,
        period: u32,
    ) -> Result<Decimal, MarketError> {
        let prices = self.get_closing_prices(symbol, timeframe, period).await?;
        let len = prices.len();
        let period_usize =
            usize::try_from(period).map_err(|_| MarketError::Parse("period too large".into()))?;
        let target_prices = &prices[len - period_usize..];

        let sum: Decimal = target_prices.iter().sum();
        let period_dec =
            Decimal::from_u32(period).ok_or(MarketError::Parse("Invalid period".into()))?;

        Ok(sum / period_dec)
    }

    async fn ema(
        &self,
        symbol: &str,
        timeframe: TimeFrame,
        period: u32,
    ) -> Result<Decimal, MarketError> {
        // 注：get_closing_prices 内部已按 CONVERGENCE_WARMUP_FACTOR 进行了预热数据拉取
        let prices = self.get_closing_prices(symbol, timeframe, period).await?;

        if prices.is_empty() {
            return Err(MarketError::Parse("Price list is empty".into()));
        }

        let mut ema = prices[0];
        let period_plus_one = Decimal::from_u32(period.saturating_add(1))
            .ok_or_else(|| MarketError::Parse("invalid period".into()))?;
        let multiplier = Decimal::from(2) / period_plus_one;

        for price in &prices[1..] {
            ema = (*price - ema) * multiplier + ema;
        }

        Ok(ema)
    }

    async fn rsi(
        &self,
        symbol: &str,
        timeframe: TimeFrame,
        period: u32,
    ) -> Result<Decimal, MarketError> {
        // 注：get_closing_prices 内部已按 CONVERGENCE_WARMUP_FACTOR 进行了预热数据拉取
        let prices = self.get_closing_prices(symbol, timeframe, period).await?;

        let period_usize =
            usize::try_from(period).map_err(|_| MarketError::Parse("period too large".into()))?;

        if prices.len() <= period_usize {
            return Err(MarketError::Parse("Insufficient data for RSI".into()));
        }

        let mut gains = Vec::new();
        let mut losses = Vec::new();

        for i in 1..prices.len() {
            let change = prices[i] - prices[i - 1];
            if change >= Decimal::ZERO {
                gains.push(change);
                losses.push(Decimal::ZERO);
            } else {
                gains.push(Decimal::ZERO);
                losses.push(change.abs());
            }
        }

        // 初始平均值
        let period_dec = Decimal::from(period);
        let mut avg_gain: Decimal = gains[..period_usize].iter().sum::<Decimal>() / period_dec;
        let mut avg_loss: Decimal = losses[..period_usize].iter().sum::<Decimal>() / period_dec;

        // 平滑计算
        let period_minus_one = Decimal::from(period - 1);

        for i in period_usize..gains.len() {
            avg_gain = (avg_gain * period_minus_one + gains[i]) / period_dec;
            avg_loss = (avg_loss * period_minus_one + losses[i]) / period_dec;
        }

        if avg_loss == Decimal::ZERO {
            return Ok(Decimal::from(100u32));
        }

        let rs = avg_gain / avg_loss;
        let hundred = Decimal::from(100u32);
        let rsi = hundred - (hundred / (Decimal::from(1) + rs));

        Ok(rsi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use okane_core::common::Stock as StockIdentity;
    use okane_core::market::entity::Candle;
    use okane_core::market::port::CandleStream;
    use okane_core::market::port::Stock;
    use okane_core::market::port::StockStatus;
    use rust_decimal_macros::dec;

    struct MockStock {
        identity: StockIdentity,
        history: Vec<Candle>,
    }

    #[async_trait]
    impl Stock for MockStock {
        fn identity(&self) -> &StockIdentity {
            &self.identity
        }
        fn current_price(&self) -> Result<Option<Decimal>, MarketError> {
            Ok(None)
        }
        fn latest_candle(&self, _tf: TimeFrame) -> Result<Option<Candle>, MarketError> {
            Ok(None)
        }
        fn last_closed_candle(&self, _tf: TimeFrame) -> Result<Option<Candle>, MarketError> {
            Ok(None)
        }
        fn subscribe(&self, _tf: TimeFrame) -> Result<CandleStream, MarketError> {
            Err(MarketError::Parse("Not implemented".into()))
        }
        async fn fetch_history(
            &self,
            _tf: TimeFrame,
            _start: chrono::DateTime<Utc>,
            _end: chrono::DateTime<Utc>,
        ) -> Result<Vec<Candle>, MarketError> {
            Ok(self.history.clone())
        }
        fn status(&self) -> StockStatus {
            StockStatus::Online
        }
    }

    struct MockMarket {
        stock: Arc<MockStock>,
    }

    #[async_trait]
    impl Market for MockMarket {
        async fn get_stock(&self, _symbol: &str) -> Result<Arc<dyn Stock>, MarketError> {
            Ok(self.stock.clone())
        }
        async fn search_symbols(
            &self,
            _query: &str,
        ) -> Result<Vec<okane_core::store::port::StockMetadata>, MarketError> {
            Ok(vec![])
        }
    }

    fn create_candles(prices: Vec<Decimal>) -> Vec<Candle> {
        prices
            .into_iter()
            .map(|p| Candle {
                time: Utc::now(),
                open: p,
                high: p,
                low: p,
                close: p,
                adj_close: None,
                volume: dec!(1000),
                is_final: true,
            })
            .collect()
    }

    #[tokio::test]
    async fn test_sma() -> anyhow::Result<()> {
        let prices = vec![dec!(10), dec!(20), dec!(30), dec!(40)];
        let stock = Arc::new(MockStock {
            identity: StockIdentity {
                symbol: "AAPL".into(),
                exchange: None,
            },
            history: create_candles(prices),
        });
        let market = Arc::new(MockMarket { stock });
        let service = MarketIndicatorService::new(market);

        // period=3, prices=[10, 20, 30, 40]
        // get_closing_prices will return all 4 candles (since 3*3=9 > 4)
        // SMA uses the last 3: [20, 30, 40] -> (20+30+40)/3 = 30
        let val = service.sma("AAPL", TimeFrame::Minute1, 3).await?;
        assert_eq!(val, dec!(30));
        Ok(())
    }

    #[tokio::test]
    async fn test_ema() -> anyhow::Result<()> {
        // Multiplier = 2 / (period + 1)
        // For period 3, multiplier = 2 / 4 = 0.5
        // Prices: [10, 20, 30]
        // EMA0 = 10
        // EMA1 = (20 - 10) * 0.5 + 10 = 15
        // EMA2 = (30 - 15) * 0.5 + 15 = 22.5
        let prices = vec![dec!(10), dec!(20), dec!(30)];
        let stock = Arc::new(MockStock {
            identity: StockIdentity {
                symbol: "AAPL".into(),
                exchange: None,
            },
            history: create_candles(prices),
        });
        let market = Arc::new(MockMarket { stock });
        let service = MarketIndicatorService::new(market);

        let val = service.ema("AAPL", TimeFrame::Minute1, 3).await?;
        assert_eq!(val, dec!(22.5));
        Ok(())
    }

    #[tokio::test]
    async fn test_rsi() -> anyhow::Result<()> {
        // Simple case: all up
        let prices = vec![dec!(10), dec!(11), dec!(12), dec!(13), dec!(14)];
        let stock = Arc::new(MockStock {
            identity: StockIdentity {
                symbol: "AAPL".into(),
                exchange: None,
            },
            history: create_candles(prices),
        });
        let market = Arc::new(MockMarket { stock });
        let service = MarketIndicatorService::new(market);

        let val = service.rsi("AAPL", TimeFrame::Minute1, 2).await?;
        // period=2.
        // changes: +1, +1, +1, +1
        // gains: [1, 1, 1, 1], losses: [0, 0, 0, 0]
        // avg_gain: (1+1)/2 = 1.0
        // avg_loss: (0+0)/2 = 0.0
        // If avg_loss is 0, RSI is 100
        assert_eq!(val, dec!(100));

        // Case with a drop
        let prices = vec![dec!(10), dec!(12), dec!(10), dec!(12), dec!(10)];
        let stock = Arc::new(MockStock {
            identity: StockIdentity {
                symbol: "AAPL".into(),
                exchange: None,
            },
            history: create_candles(prices),
        });
        let market = Arc::new(MockMarket { stock });
        let service = MarketIndicatorService::new(market);

        let val = service.rsi("AAPL", TimeFrame::Minute1, 2).await?;
        // changes: +2, -2, +2, -2
        // gains: [2, 0, 2, 0], losses: [0, 2, 0, 2]
        // Initial avg (period 2):
        // avg_gain = (2+0)/2 = 1.0
        // avg_loss = (0+2)/2 = 1.0
        // Smoothing (index 3 and 4):
        // i=2: avg_gain = (1.0*1 + 2)/2 = 1.5, avg_loss = (1.0*1 + 0)/2 = 0.5
        // i=3: avg_gain = (1.5*1 + 0)/2 = 0.75, avg_loss = (0.5*1 + 2)/2 = 1.25
        // RS = 0.75 / 1.25 = 0.6
        // RSI = 100 - (100 / (1 + 0.6)) = 100 - (100 / 1.6) = 100 - 62.5 = 37.5
        assert_eq!(val, dec!(37.5));

        Ok(())
    }

    #[tokio::test]
    async fn test_insufficient_data() -> anyhow::Result<()> {
        let prices = vec![dec!(10), dec!(20)];
        let stock = Arc::new(MockStock {
            identity: StockIdentity {
                symbol: "AAPL".into(),
                exchange: None,
            },
            history: create_candles(prices),
        });
        let market = Arc::new(MockMarket { stock });
        let service = MarketIndicatorService::new(market);

        let res = service.sma("AAPL", TimeFrame::Minute1, 3).await;
        assert!(res.is_err());
        if let Err(MarketError::Parse(msg)) = res {
            assert!(msg.contains("insufficient data"));
        } else {
            return Err(anyhow::anyhow!("Expected Parse error"));
        }
        Ok(())
    }
}
