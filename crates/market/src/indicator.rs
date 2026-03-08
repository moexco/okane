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
        let limit_i32: i32 = total_limit.try_into().unwrap_or(i32::MAX);
        let duration = timeframe.duration() * limit_i32;
        let start = end - duration;
        
        let candles = stock.fetch_history(timeframe, start, end).await?;
        let prices: Vec<Decimal> = candles.into_iter().map(|c| c.close).collect();
        
        if prices.len() < period as usize {
            return Err(MarketError::Parse(format!(
                "数据不足: 需要 {}, 实际 {}",
                period,
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
        // 注：get_closing_prices 内部已按 CONVERGENCE_WARMUP_FACTOR 进行了预热数据拉取
        let prices = self.get_closing_prices(symbol, timeframe, period).await?;
        
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
        // 注：get_closing_prices 内部已按 CONVERGENCE_WARMUP_FACTOR 进行了预热数据拉取
        let prices = self.get_closing_prices(symbol, timeframe, period).await?;
        
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
