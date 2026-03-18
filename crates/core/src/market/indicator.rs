use crate::common::TimeFrame;
use crate::market::error::MarketError;
use async_trait::async_trait;
use rust_decimal::Decimal;

/// # Summary
/// 技术指标计算服务接口。
#[async_trait]
pub trait IndicatorService: Send + Sync {
    /// 计算简单移动平均线 (SMA)
    async fn sma(
        &self,
        symbol: &str,
        timeframe: TimeFrame,
        period: u32,
    ) -> Result<Decimal, MarketError>;

    /// 计算指数移动平均线 (EMA)
    async fn ema(
        &self,
        symbol: &str,
        timeframe: TimeFrame,
        period: u32,
    ) -> Result<Decimal, MarketError>;

    /// 计算相对强弱指数 (RSI)
    async fn rsi(
        &self,
        symbol: &str,
        timeframe: TimeFrame,
        period: u32,
    ) -> Result<Decimal, MarketError>;
}
