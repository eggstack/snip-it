//! The snip-sync health and metrics HTTP control surface.
//!
//! Wire parity with the pre-migration Axum/Tower-HTTP server (proven by
//! exercising parent commit `c1c77a8` over real sockets):
//!
//! - router fallback/method responses are empty with no application
//!   content-type (`404`/`405`, `content-length: 0`); the metrics-disabled
//!   `404` keeps its `"Not found"` text application payload;
//! - known GET routes answer preflight/unsupported methods with
//!   `Allow: GET, HEAD`; unknown-path preflight short-circuits to `200` with
//!   no `Allow`;
//! - preflight responses carry CORS metadata but never the ordinary security
//!   headers; ordinary responses always carry the three security headers;
//! - every non-allow-all response carries `Vary: origin` (the historical
//!   Tower-HTTP `CorsLayer` recomputes its `Vary` from the configured rules at
//!   layer-build time: the empty configuration varies on `Origin`, while the
//!   configured and allow-all configurations use fixed origins and omit it).
//!   The retained native policy keeps conditional `Access-Control-Allow-Origin`
//!   emission, so `Vary: origin` is emitted for configured origins as well to
//!   keep the cache key coherent.

use crate::AppState;
use eggserve_primitives::{HeaderBlock, RequestHead, Response, ResponseBody, StatusCode};

/// Historical CORS cache-key token: the only `Vary` value the pre-migration
/// server ever emitted. Allow-all responses omit `Vary` entirely.
const VARY_ORIGIN: &str = "origin";

#[derive(Clone)]
pub struct CorsPolicy {
    pub allow_all: bool,
    pub allowed_origins: Vec<String>,
}

/// Create the concrete two-endpoint service. EggServe rejects request bodies
/// before this handler runs and normalizes HEAD framing at the wire boundary.
pub fn service(state: AppState, cors: CorsPolicy) -> impl eggserve_server::Service {
    eggserve_server::service_fn_head(move |head: RequestHead| {
        let state = state.clone();
        let cors = cors.clone();
        async move { handle_request(state, cors, head).await }
    })
}

async fn handle_request(
    state: AppState,
    cors: CorsPolicy,
    head: RequestHead,
) -> Result<Response, eggserve_server::ServiceError> {
    let method = head.method().as_str();
    let path = head.target().path();
    let request_headers = head.headers();

    if method == "OPTIONS" {
        let mut response_headers = Vec::new();
        if !cors.allow_all {
            response_headers.push(("vary", VARY_ORIGIN.to_owned()));
        }
        if cors.allow_all {
            response_headers.push(("access-control-allow-origin", "*".to_owned()));
            response_headers.push(("access-control-allow-methods", "*".to_owned()));
            response_headers.push(("access-control-allow-headers", "*".to_owned()));
        } else if !cors.allowed_origins.is_empty() {
            response_headers.push(("access-control-allow-methods", "GET".to_owned()));
            response_headers.push((
                "access-control-allow-headers",
                "content-type,authorization".to_owned(),
            ));
            if let Some(origin) = get_header(request_headers, "origin")
                && origin_allowed(&cors, &origin)
            {
                response_headers.push(("access-control-allow-origin", origin));
            }
        }
        if path == "/health" || path == "/metrics" {
            response_headers.push(("allow", "GET, HEAD".to_owned()));
        }
        return Ok(make_response(
            200,
            ResponseBody::Empty,
            None,
            response_headers,
        ));
    }

    let (status, payload) = match (method, path) {
        ("GET" | "HEAD", "/health") => {
            let healthy = state.db.ping().await.is_ok();
            let status = if healthy { 200 } else { 503 };
            let health = if healthy { "healthy" } else { "unhealthy" };
            let body = serde_json::to_vec(&serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "status": health,
            }))
            .map_err(|error| eggserve_server::ServiceError::internal(error.to_string()))?;
            (status, Some((body, "application/json")))
        }
        ("GET" | "HEAD", "/metrics") => {
            let (status, body, content_type) = metrics_response(&state, request_headers);
            (status, Some((body, content_type)))
        }
        (_, "/health" | "/metrics") => (405, None),
        _ => (404, None),
    };

    let mut response_headers = vec![
        ("x-content-type-options", "nosniff".to_owned()),
        ("x-frame-options", "DENY".to_owned()),
        ("cache-control", "no-store".to_owned()),
    ];
    if !cors.allow_all {
        response_headers.push(("vary", VARY_ORIGIN.to_owned()));
    }
    if status == 405 {
        response_headers.push(("allow", "GET, HEAD".to_owned()));
    }
    if let Some(origin) = get_header(request_headers, "origin")
        && origin_allowed(&cors, &origin)
    {
        response_headers.push((
            "access-control-allow-origin",
            if cors.allow_all {
                "*".to_owned()
            } else {
                origin
            },
        ));
    }
    let (body, content_type) = match payload {
        Some((body, content_type)) => (ResponseBody::Bytes(body), Some(content_type)),
        None => (ResponseBody::Empty, None),
    };
    Ok(make_response(status, body, content_type, response_headers))
}

fn metrics_response(
    state: &AppState,
    request_headers: &HeaderBlock,
) -> (u16, Vec<u8>, &'static str) {
    let (username, password) = match (
        &state.config.metrics_username,
        &state.config.metrics_password,
    ) {
        (Some(username), Some(password)) => (username.as_str(), password.as_str()),
        _ => return (404, b"Not found".to_vec(), "text/plain; charset=utf-8"),
    };

    let valid = get_header(request_headers, "authorization")
        .and_then(|value| value.strip_prefix("Basic ").map(str::to_owned))
        .and_then(|encoded| {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded).ok()
        })
        .is_some_and(|decoded| {
            use subtle::ConstantTimeEq;
            let expected = format!("{username}:{password}");
            let mut padded = decoded.clone();
            padded.resize(expected.len(), 0);
            bool::from(padded.ct_eq(expected.as_bytes())) && decoded.len() == expected.len()
        });
    if !valid {
        return (
            401,
            b"Authentication required".to_vec(),
            "text/plain; charset=utf-8",
        );
    }

    use prometheus::Encoder;
    let mut buffer = Vec::new();
    match prometheus::TextEncoder::new().encode(&state.metrics.registry.gather(), &mut buffer) {
        Ok(()) => (200, buffer, "text/plain; charset=utf-8"),
        Err(error) => (
            500,
            format!("Error gathering metrics: {error}").into_bytes(),
            "text/plain; charset=utf-8",
        ),
    }
}

fn get_header(headers: &HeaderBlock, name: &str) -> Option<String> {
    headers
        .get_first(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn origin_allowed(cors: &CorsPolicy, origin: &str) -> bool {
    cors.allow_all || cors.allowed_origins.iter().any(|allowed| allowed == origin)
}

fn make_response(
    status: u16,
    body: ResponseBody,
    content_type: Option<&str>,
    headers: Vec<(&str, String)>,
) -> Response {
    let mut builder = Response::builder().status(StatusCode::new(status).expect("valid status"));
    if let Some(content_type) = content_type {
        builder = builder
            .header("content-type", content_type)
            .expect("static content type is valid");
    }
    for (name, value) in headers {
        builder = builder
            .header(name, value)
            .expect("HTTP response header is valid");
    }
    builder.body(body).expect("response body is valid")
}
