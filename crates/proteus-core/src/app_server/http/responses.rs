use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response, StatusCode,
    header::{CONTENT_TYPE, HeaderValue},
};
use serde_json::json;

use super::HttpResponse;

pub(super) fn options_response<B>(
    request: &Request<B>,
    cors_origin: Option<HeaderValue>,
) -> HttpResponse {
    let mut response = if request
        .headers()
        .get("access-control-request-method")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|method| !matches!(method, "GET" | "POST" | "OPTIONS"))
    {
        error_response(StatusCode::METHOD_NOT_ALLOWED, "HTTP method is not allowed")
    } else {
        empty_response(StatusCode::NO_CONTENT)
    };
    add_cors_headers(&mut response, cors_origin.as_ref());
    response
}

pub(super) fn json_response<T: serde::Serialize>(status: StatusCode, body: &T) -> HttpResponse {
    match serde_json::to_vec(body) {
        Ok(body) => response_with_body(status, "application/json", Bytes::from(body)),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error:#}")),
    }
}

pub(super) fn error_response(status: StatusCode, message: &str) -> HttpResponse {
    response_with_body(
        status,
        "application/json",
        Bytes::from(
            serde_json::to_vec(&json!({
                "ok": false,
                "error": message,
            }))
            .expect("error response serializes"),
        ),
    )
}

fn empty_response(status: StatusCode) -> HttpResponse {
    response_with_body(status, "text/plain; charset=utf-8", Bytes::new())
}

pub(super) fn text_response(status: StatusCode, body: String) -> HttpResponse {
    response_with_body(status, "text/plain; charset=utf-8", Bytes::from(body))
}

fn response_with_body(status: StatusCode, content_type: &'static str, body: Bytes) -> HttpResponse {
    let body = Full::new(body).boxed_unsync();
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(body)
        .expect("HTTP response is valid")
}

pub(super) fn add_cors_headers(response: &mut HttpResponse, origin: Option<&HeaderValue>) {
    let Some(origin) = origin else {
        return;
    };
    let headers = response.headers_mut();
    headers.insert("access-control-allow-origin", origin.clone());
    headers.insert(
        "access-control-allow-methods",
        "GET, POST, OPTIONS".parse().expect("valid header"),
    );
    headers.insert(
        "access-control-allow-headers",
        "authorization, content-type, x-proteus-session, x-proteus-session-token"
            .parse()
            .expect("valid header"),
    );
    headers.insert(
        "access-control-allow-credentials",
        "true".parse().expect("valid header"),
    );
    headers.insert("vary", "origin".parse().expect("valid header"));
}
