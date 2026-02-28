//! # API 服务启动器
//!
//! 组装 axum 路由、挂载 Swagger UI、配置 CORS 并绑定 TCP 端口对外提供服务。
//! 本模块不直接启动 `main()`, 而是由 `crates/app` 的 DI 容器持有并调用。

use std::sync::Arc;

use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_swagger_ui::SwaggerUi;

use okane_core::store::port::SystemStore;
use okane_core::trade::port::TradePort;
use okane_manager::strategy::StrategyManager;

use crate::routes::{account, admin, auth, strategy, market, watchlist, trade};

// ============================================================
//  共享应用状态
// ============================================================

/// 全局应用状态，通过 axum 的 `State` 提取器注入到每个 Handler 中。
///
/// # Invariants
/// - `strategy_manager` 和 `trade_port` 在服务启动前由 DI 容器注入，生命周期与进程等同。
#[derive(Clone)]
pub struct AppState {
    /// 策略管理器 (Facade)
    pub strategy_manager: Arc<StrategyManager>,
    /// 交易服务端口 (用于账户快照查询)
    pub trade_port: Arc<dyn TradePort>,
    /// 系统数据访问接口 (用于鉴权验证和用户管理)
    pub system_store: Arc<dyn SystemStore>,
}

// ============================================================
//  OpenAPI 文档定义
// ============================================================

/// 全局 OpenAPI 文档结构
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Okane 量化引擎 API",
        version = "0.1.0",
        description = "Okane 量化交易引擎的 RESTful API 网关。提供账户资产查询、策略管理、和系统配置功能。",
        contact(name = "Okane Team"),
        license(name = "MIT")
    ),
    tags(
        (name = "鉴权 (Auth)", description = "JWT 获取、密码修改登录认证相关API"),
        (name = "系统管理 (Admin)", description = "用户开户、全站核心系统管理API"),
        (name = "账户 (Account)", description = "系统账户资产与持仓查询"),
        (name = "策略 (Strategy)", description = "策略的部署、停止、查询与源码管理")
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

/// 为 OpenAPI 文档注入全局 Bearer JWT 鉴权方案。
///
/// 注册后，Swagger UI 页面顶部将显示 🔒 Authorize 按钮，
/// 用户可以填入 JWT Token 后对所有标记了 `security` 的接口进行鉴权测试。
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        // 若 components 不存在则创建
        let components = openapi.components.get_or_insert_with(Default::default);

        // 注册名为 "bearer_jwt" 的 HTTP Bearer 鉴权方案
        components.add_security_scheme(
            "bearer_jwt",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .description(Some(
                        "在此处填入登录接口返回的 JWT Token（无需 'Bearer ' 前缀）",
                    ))
                    .build(),
            ),
        );
    }
}

// ============================================================
//  服务构建与启动
// ============================================================

/// 构建完整的 axum 应用路由树并启动 HTTP 监听。
///
/// # Arguments
/// * `state` - 由外部 DI 容器注入的共享状态
/// * `bind_addr` - 监听的地址与端口，如 `"0.0.0.0:8080"`
///
/// # Panics
/// 如果 TCP 绑定失败将 panic (生产环境应有优雅降级)。
pub async fn start_server(state: AppState, bind_addr: &str) -> Result<(), Box<dyn std::error::Error>> {
    // 1. 无需鉴权的公开路由
    let public_router = OpenApiRouter::new()
        .routes(routes!(auth::login))
        .routes(routes!(market::search_stocks))
        .routes(routes!(market::get_candles));

    // 2. 只需要合法 JWT 鉴权的路由 (普通用户)
    let user_protected_router = OpenApiRouter::new()
        .routes(routes!(auth::change_password))
        .routes(routes!(account::get_account_snapshot))
        .routes(routes!(strategy::list_strategies))
        .routes(routes!(strategy::get_strategy))
        .routes(routes!(strategy::deploy_strategy))
        .routes(routes!(strategy::stop_strategy))
        .routes(routes!(watchlist::get_watchlist))
        .routes(routes!(watchlist::add_to_watchlist))
        .routes(routes!(watchlist::remove_from_watchlist))
        .routes(routes!(trade::get_orders))
        .routes(routes!(trade::place_order))
        .routes(routes!(trade::cancel_order))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::auth::auth_middleware,
        ));

    // 3. 需要 Admin 角色鉴权的路由
    let admin_protected_router = OpenApiRouter::new()
        .routes(routes!(admin::create_user))
        .routes(routes!(admin::update_settings))
        .layer(axum::middleware::from_fn(
            crate::middleware::auth::require_admin,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::auth::auth_middleware,
        ));

    // 4. 合并所有路由与自动收集的 OpenAPI Doc
    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(public_router)
        .merge(user_protected_router)
        .merge(admin_protected_router)
        .with_state(state)
        .split_for_parts();

    // 5. 配置 CORS (开发阶段允许所有来源)
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // 3. 合并 Swagger UI 路由并应用中间件
    let app: Router = router
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", api))
        .layer(cors);

    // 4. 绑定端口并启动
    tracing::info!("🚀 Okane API Server listening on {}", bind_addr);
    tracing::info!("📖 Swagger UI: http://{}/swagger-ui/", bind_addr);

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
