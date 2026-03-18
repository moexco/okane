use axum::{body::Body, http::Request, middleware::Next, response::Response};
use http_body_util::BodyExt;
use serde_json::Value;
use std::time::Instant;

pub async fn timer_middleware(req: Request<Body>, next: Next) -> Response {
    let start = Instant::now();
    let path = req.uri().path().to_string();
    let method = req.method().to_string();

    let response = next.run(req).await;
    let elapsed_millis = start.elapsed().as_millis();
    #[allow(clippy::manual_unwrap_or)]
    let elapsed = if elapsed_millis > u128::from(u64::MAX) {
        u64::MAX
    } else {
        match u64::try_from(elapsed_millis) {
            Ok(value) => value,
            Err(_) => u64::MAX,
        }
    };

    tracing::info!(
        "API Request: {} {} - Completed in {}ms",
        method,
        path,
        elapsed
    );

    let (mut parts, body) = response.into_parts();

    // 1. 尝试从 Extension 中获取待序列化的原始数据 (ErasedResponse)
    // 这是核心优化：避免 Handler 序列化一次，中间件再反序列化+注入+序列化
    if let Some(erased) = parts
        .extensions
        .remove::<std::sync::Arc<dyn crate::types::ErasedResponse>>()
    {
        let status = erased.status();
        let bytes = erased.render(elapsed);
        parts.status = status;
        parts.headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/json"),
        );
        return Response::from_parts(parts, Body::from(bytes));
    }

    // 2. 兼容逻辑：处理那些仍然直接返回 Json<ApiResponse<T>> 的旧 Handler
    let content_type = parts.headers.get(axum::http::header::CONTENT_TYPE);
    let is_json = content_type
        .and_then(|h| h.to_str().ok())
        .map(|s| s.contains("application/json"))
        .unwrap_or(false);

    if is_json {
        // Collect body bytes
        let bytes = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(_) => return Response::from_parts(parts, Body::empty()),
        };

        // Try to inject latency_ms into JSON (这是旧的耗时方式)
        if let Ok(mut json) = serde_json::from_slice::<Value>(&bytes) {
            if let Some(obj) = json.as_object_mut() {
                // Only inject if it looks like our standard ApiResponse or ApiErrorResponse
                if obj.contains_key("success") {
                    obj.insert("latency_ms".to_string(), Value::from(elapsed));

                    if let Ok(new_bytes) = serde_json::to_vec(&json) {
                        return Response::from_parts(parts, Body::from(new_bytes));
                    }
                }
            }
        }

        Response::from_parts(parts, Body::from(bytes))
    } else {
        Response::from_parts(parts, body)
    }
}
