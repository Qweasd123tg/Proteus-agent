use std::time::Duration;

pub(super) async fn send_with_transport_retry<F>(
    mut build: F,
) -> Result<reqwest::Response, reqwest::Error>
where
    F: FnMut() -> reqwest::RequestBuilder,
{
    const MAX_RETRIES: usize = 2;

    let mut attempt = 0usize;
    loop {
        match build().send().await {
            Ok(response) => return Ok(response),
            Err(error) if attempt < MAX_RETRIES && should_retry_transport_error(&error) => {
                attempt += 1;
                tokio::time::sleep(retry_delay(attempt)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn should_retry_transport_error(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_timeout() || error.is_body()
}

fn retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(150 * attempt as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn classifies_connect_error_as_retryable() {
        let error = reqwest::Client::new()
            .get("http://127.0.0.1:1")
            .send()
            .await
            .expect_err("port 1 should reject connection in test env");

        assert!(should_retry_transport_error(&error));
    }
}
