//! # 回测 API 路由控制器
//!
//! 实现 `/api/v1/user/backtest` 路径下的 REST 接口。
//! 对应 UI 原型中的 Backtest 功能模块。

use axum::extract::State;
use chrono::{DateTime, Utc};

use crate::error::ApiError;
use crate::middleware::auth::CurrentUser;
use crate::server::AppState;
use crate::types::{ApiResponse, ApiResult, BacktestRequest, BacktestResponse};

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
    axum::Json(req): axum::Json<BacktestRequest>,
) -> Result<ApiResult<BacktestResponse>, ApiError> {
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
        .map_err(|e| ApiError::BadRequest(format!("invalid start time format: {}", e)))?
        .with_timezone(&Utc);

    let end_time = DateTime::parse_from_rfc3339(&req.end)
        .map_err(|e| ApiError::BadRequest(format!("invalid end time format: {}", e)))?
        .with_timezone(&Utc);

    // 校验时间范围
    if start_time >= end_time {
        return Err(ApiError::BadRequest(
            "start time must be before end time".to_string(),
        ));
    }

    // 解析初始资金
    let initial_balance = Decimal::from_str(&req.initial_balance)
        .map_err(|_| ApiError::BadRequest("invalid initial balance value".to_string()))?;

    // Base64 解码源码
    use base64::prelude::{BASE64_STANDARD, Engine as _};
    let source = BASE64_STANDARD
        .decode(&req.source_base64)
        .map_err(|e| ApiError::BadRequest(format!("base64 decode failed: {}", e)))?;

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
        .map_err(|e| ApiError::runtime(format!("backtest execution failed: {}", e)))?;

    // 转换结果为 Response
    let ret = BacktestResponse {
        final_snapshot: result.final_snapshot.into(),
        trades: result.trades.into_iter().map(Into::into).collect(),
        candle_count: result.candle_count,
    };

    Ok(ApiResult(ret))
}
