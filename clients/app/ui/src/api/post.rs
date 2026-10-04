use super::*;

pub(crate) struct PostError {
    pub(crate) message: String,
    pub(crate) rejected_before_admission: bool,
}

impl From<String> for PostError {
    fn from(message: String) -> Self {
        Self {
            message,
            rejected_before_admission: true,
        }
    }
}

impl PostError {
    fn unconfirmed(message: String) -> Self {
        Self {
            message,
            rejected_before_admission: false,
        }
    }

    fn http(status: u16, message: String) -> Self {
        Self {
            message,
            // A timeout or server failure may happen after the command was admitted.
            rejected_before_admission: (400..500).contains(&status) && status != 408,
        }
    }
}

pub(crate) async fn post_json<T: Serialize>(path: &str, body: &T) -> Result<StdioOutput, String> {
    post_json_for_admission(path, body)
        .await
        .map_err(|error| error.message)
}

pub(crate) async fn post_json_for_admission<T: Serialize>(
    path: &str,
    body: &T,
) -> Result<StdioOutput, PostError> {
    let request_body = serde_json::to_string(body).map_err(|error| error.to_string())?;
    let text = post_text(path, &request_body, None).await?;
    serde_json::from_str(&text)
        .map_err(|error| PostError::unconfirmed(format!("invalid response JSON: {error}")))
}

pub(crate) async fn post_text_with_signal(
    path: &str,
    body: &str,
    signal: Option<&web_sys::AbortSignal>,
) -> Result<String, String> {
    post_text(path, body, signal)
        .await
        .map_err(|error| error.message)
}

async fn post_text(
    path: &str,
    body: &str,
    signal: Option<&web_sys::AbortSignal>,
) -> Result<String, PostError> {
    let init = RequestInit::new();
    init.set_method("POST");
    init.set_mode(RequestMode::Cors);
    init.set_signal(signal);
    init.set_body(&JsValue::from_str(body));
    let headers = Headers::new().map_err(js_error)?;
    headers
        .set("content-type", "application/json")
        .map_err(js_error)?;
    set_authorization_header(&headers, &current_session_token())?;
    init.set_headers(headers.as_ref());
    let request = Request::new_with_str_and_init(&app_server_url(path), &init).map_err(js_error)?;
    let window = window().ok_or_else(|| "window is unavailable".to_owned())?;
    let value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|error| PostError::unconfirmed(js_error(error)))?;
    let response = value
        .dyn_into::<Response>()
        .map_err(|error| PostError::unconfirmed(js_error(error)))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| PostError::http(status, js_error(error)))?;
    let text = JsFuture::from(body)
        .await
        .map_err(|error| PostError::http(status, js_error(error)))?
        .as_string()
        .ok_or_else(|| PostError::http(status, "response body is not text".to_owned()))?;
    if !response.ok() {
        return Err(PostError::http(status, http_error(status, &text)));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_failure_classification() {
        for status in [400, 401, 403, 404, 409, 413, 422, 429] {
            assert!(PostError::http(status, String::new()).rejected_before_admission);
        }
        for status in [200, 408, 500, 503] {
            assert!(!PostError::http(status, String::new()).rejected_before_admission);
        }
        assert!(!PostError::unconfirmed("connection lost".to_owned()).rejected_before_admission);
    }
}
