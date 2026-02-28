//! # `okane-api` - HTTP API 网关
//!
//! 本 crate 是 Okane 量化交易引擎的 HTTP/REST 服务入口。
//! 使用 `axum` 构建路由与控制器，通过 `utoipa` 自动生成 OpenAPI 3.0 Swagger 文档。
//!
//! ## 架构职责
//! - 接收来自 Flutter 客户端或浏览器的 HTTP 请求
//! - 执行 JWT 鉴权后分发至 User / Admin 路由组
//! - 调用下层 `StrategyManager` 和 `TradePort` 完成业务操作
//! - 将领域模型转换为 DTO 返回给前端

pub mod types;
pub mod error;
pub mod middleware;
pub mod routes;
pub mod server;
