use std::{borrow::Cow, sync::Arc};

use hyper::{
    Method, Request, StatusCode,
    header::{AUTHORIZATION, HeaderValue, ORIGIN},
};

use super::{HttpResponse, config::HttpServerConfig, error_response};

const SESSION_TOKEN_HEADERS: [&str; 2] = ["x-proteus-session", "x-proteus-session-token"];
const SESSION_TOKEN_QUERY: &str = "token";
const SESSION_TOKEN_QUERY_ALIASES: [&str; 3] = ["session", "session_token", "proteus_session"];

#[derive(Clone)]
pub(super) struct HttpSecurity {
    pub(super) session_token: Arc<str>,
    pub(super) require_session_token: bool,
    pub(super) allowed_origins: Arc<[String]>,
}

impl HttpSecurity {
    pub(super) fn from_config(config: &HttpServerConfig) -> Self {
        Self {
            session_token: Arc::from(config.session_token.as_str()),
            require_session_token: config.require_session_token,
            allowed_origins: Arc::from(config.allowed_origins.clone().into_boxed_slice()),
        }
    }
}

pub(super) fn endpoint_requires_auth(method: &Method, path: &str) -> bool {
    !matches!(
        (method, path),
        (&Method::OPTIONS, _) | (&Method::GET, "/health")
    )
}

pub(super) fn request_requires_session_token(
    method: &Method,
    path: &str,
    security: &HttpSecurity,
) -> bool {
    security.require_session_token && endpoint_requires_auth(method, path)
}

pub(super) fn validate_origin<B>(
    request: &Request<B>,
    security: &HttpSecurity,
) -> Result<Option<HeaderValue>, Box<HttpResponse>> {
    let Some(origin) = request.headers().get(ORIGIN) else {
        return Ok(None);
    };
    let Ok(origin_text) = origin.to_str() else {
        return Err(Box::new(error_response(
            StatusCode::FORBIDDEN,
            "request origin is not allowed",
        )));
    };
    if is_allowed_origin(origin_text, &security.allowed_origins) {
        return Ok(Some(origin.clone()));
    }
    Err(Box::new(error_response(
        StatusCode::FORBIDDEN,
        "request origin is not allowed",
    )))
}

fn is_allowed_origin(origin: &str, allowed_origins: &[String]) -> bool {
    allowed_origins
        .iter()
        .any(|allowed| origin.eq_ignore_ascii_case(allowed))
}

pub(super) fn request_has_valid_token<B>(request: &Request<B>, security: &HttpSecurity) -> bool {
    SESSION_TOKEN_HEADERS.iter().any(|header| {
        request
            .headers()
            .get(*header)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|token| token_matches(token, &security.session_token))
    }) || request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token)
        .is_some_and(|token| token_matches(token, &security.session_token))
        || request
            .uri()
            .query()
            .is_some_and(|query| query_has_valid_token(query, &security.session_token))
}

fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() {
        Some(token)
    } else {
        None
    }
}

fn query_has_valid_token(query: &str, expected: &str) -> bool {
    query.split('&').any(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_decode_query_value(value);
        (key == SESSION_TOKEN_QUERY || SESSION_TOKEN_QUERY_ALIASES.contains(&key))
            && token_matches(value.as_ref(), expected)
    })
}

fn percent_decode_query_value(value: &str) -> Cow<'_, str> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut changed = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                if let Some(byte) = hex_pair(bytes[index + 1], bytes[index + 2]) {
                    decoded.push(byte);
                    changed = true;
                    index += 3;
                } else {
                    decoded.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    if changed {
        String::from_utf8(decoded)
            .map(Cow::Owned)
            .unwrap_or(Cow::Borrowed(value))
    } else {
        Cow::Borrowed(value)
    }
}

fn hex_pair(high: u8, low: u8) -> Option<u8> {
    Some(hex_digit(high)? << 4 | hex_digit(low)?)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn token_matches(provided: &str, expected: &str) -> bool {
    let provided = provided.as_bytes();
    let expected = expected.as_bytes();
    if provided.len() != expected.len() {
        return false;
    }
    provided
        .iter()
        .zip(expected.iter())
        .fold(0_u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}
