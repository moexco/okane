use axum::{
    body::Body,
    http::Request,
    middleware::Next,
    response::Response,
};
use http_body_util::BodyExt;
use std::time::Instant;
use serde_json::Value;

pub async fn timer_middleware(req: Request<Body>, next: Next) -> Response {
    let start = Instant::now();
    let path = req.uri().path().to_string();
    let method = req.method().to_string();

    let response = next.run(req).await;
    let elapsed = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

    tracing::info!("API Request: {} {} - Completed in {}ms", method, path, elapsed);

    let (parts, body) = response.into_parts();

    // Check if it's JSON
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

        // Try to inject latency_ms into JSON
        if let Ok(mut json) = serde_json::from_slice::<Value>(&bytes) {
            if let Some(obj) = json.as_object_mut() {
                // Only inject if it looks like our standard ApiResponse or ApiErrorResponse (has 'success' field)
                if obj.contains_key("success") {
                    obj.insert("latency_ms".to_string(), Value::from(elapsed));
                    
                    if let Ok(new_bytes) = serde_json::to_vec(&json) {
                        return Response::from_parts(parts, Body::from(new_bytes));
                    }
                }
            }
        }
        
        // Return original if injection fails
        Response::from_parts(parts, Body::from(bytes))
    } else {
        Response::from_parts(parts, body)
    }
}
