//! # 回测 API 路由控制器
//!
//! 实现 `/api/v1/user/backtest` 路径下的 REST 接口。
//! 对应 UI 原型中的 Backtest 功能模块。

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Utc};

use crate::error::ApiError;
use crate::middleware::auth::CurrentUser;
use crate::server::AppState;
use crate::types::{ApiResponse, BacktestRequest, BacktestResponse};

// ============================================================
//  Handler 实现
// ============================================================

/// 执行策略回测
///
/// 传入策略源码与回测参数，在引擎的隔离环境中运行策略，并返回最终资产快照与完整交易流水。
#[utoipa::path(
    post,
    path = "/api/v1/user/backtest",
    tag = "策略 (Strategy)",
    security(("bearer_jwt" = [])),
    request_body = BacktestRequest,
    responses(
        (status = 200, description = "回测执行成功，返回结果数据", body = ApiResponse<BacktestResponse>),
        (status = 400, description = "请求参数错误或数据不足"),
        (status = 401, description = "未认证")
    )
)]
pub async fn run_backtest(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    Json(req): Json<BacktestRequest>,
) -> Result<Json<ApiResponse<BacktestResponse>>, ApiError> {
    use okane_core::common::TimeFrame;
    use okane_core::strategy::entity::EngineType;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    // 解析时间周期
    let timeframe: TimeFrame = req
        .timeframe
        .parse()
        .map_err(|e: String| ApiError::BadRequest(e))?;

    // 解析引擎类型
    let engine_type: EngineType = req
        .engine_type
        .parse()
        .map_err(|e: String| ApiError::BadRequest(e))?;

    // 解析起止时间
    let start_time = DateTime::parse_from_rfc3339(&req.start)
        .map_err(|e| ApiError::BadRequest(format!("无效的开始时间格式: {}", e)))?
        .with_timezone(&Utc);

    let end_time = DateTime::parse_from_rfc3339(&req.end)
        .map_err(|e| ApiError::BadRequest(format!("无效的结束时间格式: {}", e)))?
        .with_timezone(&Utc);

    // 校验时间范围
    if start_time >= end_time {
        return Err(ApiError::BadRequest("开始时间必须早于结束时间".to_string()));
    }

    // 解析初始资金
    let initial_balance = Decimal::from_str(&req.initial_balance)
        .map_err(|_| ApiError::BadRequest("无效的初始资金数值".to_string()))?;

    // Base64 解码源码
    let source = base64_decode(&req.source_base64)
        .map_err(|e| ApiError::BadRequest(format!("Base64 解码失败: {}", e)))?;

    // 构建 Runner 请求
    let run_req = okane_manager::backtest::BacktestRequest {
        symbol: req.symbol,
        timeframe,
        start: start_time,
        end: end_time,
        engine_type,
        source,
        initial_balance,
    };

    // 执行回测
    let result = state
        .backtest_runner
        .run(run_req)
        .await
        .map_err(|e| ApiError::Internal(format!("回测执行失败: {}", e)))?;

    // 转换结果为 Response
    let ret = BacktestResponse {
        final_snapshot: result.final_snapshot.into(),
        trades: result.trades.into_iter().map(Into::into).collect(),
        candle_count: result.candle_count,
    };

    Ok(Json(ApiResponse::ok(ret)))
}

// ============================================================
//  辅助函数
// ============================================================

/// 简单的 Base64 解码 (不引入额外依赖)
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let input = input.trim_end_matches('=');
    let mut output = Vec::with_capacity(input.len() * 3 / 4);

    let mut buf: u32 = 0;
    let mut bits: u32 = 0;

    for byte in input.bytes() {
        let val = TABLE
            .iter()
            .position(|&b| b == byte)
            .ok_or_else(|| format!("非法 Base64 字符: {}", byte as char))?
            as u32;
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }

    Ok(output)
}
