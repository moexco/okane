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

impl MarketIndicatorService {
    pub fn new(market: Arc<dyn Market>) -> Self {
        Self { market }
    }

    async fn get_closing_prices(
        &self,
        symbol: &str,
        timeframe: TimeFrame,
        limit: u32,
    ) -> Result<Vec<Decimal>, MarketError> {
        let stock = self.market.get_stock(symbol).await?;
        let end = Utc::now();
        // 取足够长的数据以计算指标 (假设取 limit * 3 的长度以保证收敛)
        let limit_i32: i32 = limit.try_into().unwrap_or(i32::MAX);
        let duration = timeframe.duration() * limit_i32.saturating_mul(3);
        let start = end - duration;
        
        let candles = stock.fetch_history(timeframe, start, end).await?;
        let prices: Vec<Decimal> = candles.into_iter().map(|c| c.close).collect();
        
        if prices.len() < limit as usize {
            return Err(MarketError::Parse(format!(
                "数据不足: 需要 {}, 实际 {}",
                limit,
                prices.len()
            )));
        }
        
        Ok(prices)
    }
}

#[async_trait]
impl IndicatorService for MarketIndicatorService {
    async fn sma(&self, symbol: &str, timeframe: TimeFrame, period: u32) -> Result<Decimal, MarketError> {
        let prices = self.get_closing_prices(symbol, timeframe, period).await?;
        let len = prices.len();
        let target_prices = &prices[len - period as usize..];
        
        let sum: Decimal = target_prices.iter().sum();
        let period_dec = Decimal::from_u32(period).ok_or(MarketError::Parse("Invalid period".into()))?;
        
        Ok(sum / period_dec)
    }

    async fn ema(&self, symbol: &str, timeframe: TimeFrame, period: u32) -> Result<Decimal, MarketError> {
        // 为了使 EMA 收敛，取更多数据
        let limit = period * 2;
        let prices = self.get_closing_prices(symbol, timeframe, limit).await?;
        
        if prices.is_empty() {
             return Err(MarketError::Parse("Price list is empty".into()));
        }

        let mut ema = prices[0];
        let multiplier = Decimal::from_f64(2.0 / (f64::from(period) + 1.0))
            .ok_or_else(|| MarketError::Parse("Failed to calculate EMA multiplier".into()))?;

        for price in &prices[1..] {
            ema = (*price - ema) * multiplier + ema;
        }

        Ok(ema)
    }

    async fn rsi(&self, symbol: &str, timeframe: TimeFrame, period: u32) -> Result<Decimal, MarketError> {
        let limit = period * 2;
        let prices = self.get_closing_prices(symbol, timeframe, limit).await?;
        
        if prices.len() <= period as usize {
            return Err(MarketError::Parse("Insufficient data for RSI".into()));
        }

        let mut gains = Vec::new();
        let mut losses = Vec::new();

        for i in 1..prices.len() {
            let change = prices[i] - prices[i-1];
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
        let mut avg_gain: Decimal = gains[..period as usize].iter().sum::<Decimal>() / period_dec;
        let mut avg_loss: Decimal = losses[..period as usize].iter().sum::<Decimal>() / period_dec;

        // 平滑计算
        let period_minus_one = Decimal::from(period - 1);

        for i in period as usize..gains.len() {
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
