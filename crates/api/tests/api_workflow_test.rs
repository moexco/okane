use reqwest::StatusCode;
use utoipa::OpenApi;
use okane_api::types::{
    ApiResponse, ChangePasswordRequest, CreateUserRequest, LoginRequest, LoginResponse,
    StartStrategyRequest, StrategyResponse,
};
use okane_api::server::AppState;
use okane_core::store::port::{SystemStore, User, UserRole};
use okane_market::manager::MarketImpl;
use okane_store::market::SqliteMarketStore;
use okane_store::strategy::SqliteStrategyStore;
use okane_store::system::SqliteSystemStore;
use okane_trade::account::AccountManager;
use okane_trade::matcher::LocalMatchEngine;
use okane_trade::service::TradeService;
use okane_engine::factory::EngineFactory;
use okane_feed::yahoo::YahooProvider;
use okane_manager::strategy::StrategyManager;
use std::sync::Arc;
use tokio::net::TcpListener;

// 帮助函数：在随机端口启动测试服务器
async fn spawn_test_server() -> (String, Arc<dyn SystemStore>, tempfile::TempDir) {
    let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    okane_store::config::set_root_dir(tmp_dir.path().to_path_buf());


    let system_store: Arc<dyn SystemStore> = Arc::new(SqliteSystemStore::new().await.unwrap());
    
    // 强制覆盖 admin 的密码为已知测试密码 "test_admin_pwd"
    let hashed = bcrypt::hash("test_admin_pwd", bcrypt::DEFAULT_COST).unwrap();
    let admin_user = User {
        id: "admin".to_string(),
        name: "Admin".to_string(),
        password_hash: hashed,
        role: UserRole::Admin,
        force_password_change: true,
        created_at: chrono::Utc::now(),
    };
    system_store.save_user(&admin_user).await.unwrap();

    let feed = Arc::new(YahooProvider::new());
    let market_store = Arc::new(SqliteMarketStore::new().unwrap());
    let strategy_store = Arc::new(SqliteStrategyStore::new().unwrap());
    let market = MarketImpl::new(feed, market_store);
    let engine_builder = Arc::new(EngineFactory::new(market.clone()));
    let account_manager = Arc::new(AccountManager::default());
    account_manager.ensure_account_exists(
        okane_core::trade::entity::AccountId("SysAcct_1".to_string()),
        rust_decimal::Decimal::new(10_000_000, 2), // $100k test money
    );
    let pending_port = Arc::new(okane_store::pending_order::MemoryPendingOrderStore::new());
    let matcher = std::sync::Arc::new(LocalMatchEngine::new(rust_decimal::Decimal::ZERO));
    let trade_service = Arc::new(TradeService::new(account_manager, matcher, market.clone(), pending_port));

    let strategy_manager = StrategyManager::new(
        strategy_store,
        engine_builder,
        trade_service.clone(),
    );

    let app_config = Arc::new(okane_core::config::AppConfig::default());

    let state = AppState {
        strategy_manager,
        trade_port: trade_service,
        system_store: system_store.clone(),
        market_port: market,
        app_config,
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let addr = format!("http://127.0.0.1:{}", port);
    
    // 我们不能直接传递 port 给 start_server，因为 start_server 内部也试图 bind
    // 修改策略：重写在文件中的提取逻辑，使用该 listener 启动服务
    let (router, _api) = utoipa_axum::router::OpenApiRouter::with_openapi(
        okane_api::server::ApiDoc::openapi(),
    )
    .merge(
        utoipa_axum::router::OpenApiRouter::new()
            .routes(utoipa_axum::routes!(okane_api::routes::auth::login))
    )
    .merge(
        utoipa_axum::router::OpenApiRouter::new()
            .routes(utoipa_axum::routes!(okane_api::routes::auth::change_password))
            .routes(utoipa_axum::routes!(okane_api::routes::account::get_account_snapshot))
            .routes(utoipa_axum::routes!(okane_api::routes::strategy::list_strategies))
            .routes(utoipa_axum::routes!(okane_api::routes::strategy::get_strategy))
            .routes(utoipa_axum::routes!(okane_api::routes::strategy::deploy_strategy))
            .routes(utoipa_axum::routes!(okane_api::routes::strategy::stop_strategy))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                okane_api::middleware::auth::auth_middleware,
            )),
    )
    .merge(
        utoipa_axum::router::OpenApiRouter::new()
            .routes(utoipa_axum::routes!(okane_api::routes::admin::create_user))
            .layer(axum::middleware::from_fn(
                okane_api::middleware::auth::require_admin,
            ))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                okane_api::middleware::auth::auth_middleware,
            )),
    )
    .with_state(state)
    .split_for_parts();

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    
    // 稍微等待服务器启动
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    (addr, system_store, tmp_dir)
}

#[tokio::test]
async fn test_full_api_workflow() {
    let _ = tracing_subscriber::fmt().with_env_filter("debug").try_init();
    
    let (base_url, _store, _tmp) = spawn_test_server().await;
    let client = reqwest::Client::new();

    // ============================================
    // Case 1: 登录失败 (密码错误)
    // ============================================
    let res = client
        .post(format!("{}/api/v1/auth/login", base_url))
        .json(&LoginRequest {
            username: "admin".to_string(),
            password: "wrongpassword".to_string(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // ============================================
    // Case 2: 成功登录 Admin
    // ============================================
    let res = client
        .post(format!("{}/api/v1/auth/login", base_url))
        .json(&LoginRequest {
            username: "admin".to_string(),
            password: "test_admin_pwd".to_string(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let login_data: ApiResponse<LoginResponse> = res.json().await.unwrap();
    let admin_token = login_data.data.unwrap().token;

    // ============================================
    // Case 3: 强制锁定 (Force Password Change Lock)
    // ============================================
    let res = client
        .get(format!("{}/api/v1/user/strategies", base_url))
        .bearer_auth(&admin_token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "未改密码即访问业务被拒绝");

    // ============================================
    // Case 4: 修改密码成功
    // ============================================
    let res = client
        .post(format!("{}/api/v1/auth/change_password", base_url))
        .bearer_auth(&admin_token)
        .json(&ChangePasswordRequest {
            old_password: "test_admin_pwd".to_string(),
            new_password: "new_secure_password".to_string(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 重新登录获取新 Token (原本的 Token 其实还有效，但重登更真实)
    let res = client
        .post(format!("{}/api/v1/auth/login", base_url))
        .json(&LoginRequest {
            username: "admin".to_string(),
            password: "new_secure_password".to_string(),
        })
        .send()
        .await
        .unwrap();
    let login_data: ApiResponse<LoginResponse> = res.json().await.unwrap();
    let admin_token_new = login_data.data.unwrap().token;

    // ============================================
    // Case 5: 创建新用户 (Admin)
    // ============================================
    let pre_check = _store.get_user("trader_01").await;
    tracing::info!("Pre-check trader_01 in DB before API call: {:?}", pre_check.unwrap().map(|u| u.id));

    let res = client
        .post(format!("{}/api/v1/admin/users", base_url))
        .bearer_auth(&admin_token_new)
        .json(&CreateUserRequest {
            id: "trader_01".to_string(),
            name: "Trader One".to_string(),
            password: "trader_password".to_string(),
            role: "Standard".to_string(),
        })
        .send()
        .await
        .unwrap();
    let status = res.status();
    let body_text = res.text().await.unwrap();
    tracing::info!("CreateUser status: {}, body: {}", status, body_text);
    assert_eq!(status, StatusCode::OK, "Expected 200 OK but got {}: {}", status, body_text);

    // ============================================
    // Case 6: 权限隔离 - Trader 尝试创建账户 (Forbidden)
    // ============================================
    let res = client
        .post(format!("{}/api/v1/auth/login", base_url))
        .json(&LoginRequest {
            username: "trader_01".to_string(),
            password: "trader_password".to_string(),
        })
        .send()
        .await
        .unwrap();
    let trader_login: ApiResponse<LoginResponse> = res.json().await.unwrap();
    let trader_token = trader_login.data.unwrap().token;

    // 需先改密码
    client
        .post(format!("{}/api/v1/auth/change_password", base_url))
        .bearer_auth(&trader_token)
        .json(&ChangePasswordRequest {
            old_password: "trader_password".to_string(),
            new_password: "trader_new_pwd".to_string(),
        })
        .send()
        .await
        .unwrap();

    // Trader 尝试当 Admin
    let res = client
        .post(format!("{}/api/v1/admin/users", base_url))
        .bearer_auth(&trader_token)
        .json(&CreateUserRequest {
            id: "hacker".to_string(),
            name: "Hacker".to_string(),
            password: "hacker".to_string(),
            role: "Admin".to_string(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "非管理员无法创建账户");

    // ============================================
    // Case 7: 业务流程 - 策略部署 (Strategy Deployment)
    // ============================================
    let deploy_res = client
        .post(format!("{}/api/v1/user/strategies", base_url))
        .bearer_auth(&trader_token)
        .json(&StartStrategyRequest {
            symbol: "AAPL".to_string(),
            account_id: "SysAcct_1".to_string(),
            timeframe: "1m".to_string(),
            engine_type: "JavaScript".to_string(),
            source_base64: "Y29uc29sZS5sb2coJ2hlbGxvJyk7".to_string(), // console.log('hello');
        })
        .send()
        .await
        .unwrap();
    assert_eq!(deploy_res.status(), StatusCode::OK);
    
    let deploy_data: ApiResponse<String> = deploy_res.json().await.unwrap();
    let strategy_id = deploy_data.data.unwrap();

    // 检查列表
    let list_res = client
        .get(format!("{}/api/v1/user/strategies", base_url))
        .bearer_auth(&trader_token)
        .send()
        .await
        .unwrap();
    let list_data: ApiResponse<Vec<StrategyResponse>> = list_res.json().await.unwrap();
    let strategies = list_data.data.unwrap();
    assert_eq!(strategies.len(), 1);
    assert_eq!(strategies[0].id, strategy_id);
    
    // ============================================
    // Case 8: 账户快照获取
    // ============================================
    let acc_res = client
        .get(format!("{}/api/v1/user/account/SysAcct_1", base_url))
        .bearer_auth(&trader_token)
        .send()
        .await
        .unwrap();
    assert_eq!(acc_res.status(), StatusCode::OK);
}
