//! Provider-owned ChatGPT OAuth. See model-pack/UPSTREAM.md for the source boundary.
use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::Value;

pub mod cli;
mod login;
mod oauth;
mod store;
#[cfg(test)]
mod tests;

pub(super) const CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const ISSUER: &str = "https://auth.openai.com";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const LOGIN_HINT: &str = "proteus-reference-worker auth openai_codex login";

#[derive(Clone, Debug)]
pub(super) struct CodexAuth {
    path: PathBuf,
    oauth: oauth::OAuthClient,
}

pub(super) struct Access {
    pub token: String,
    pub headers: HeaderMap,
}

impl CodexAuth {
    pub fn from_config(config: &Value) -> Result<Self> {
        let path = match config.get("auth_file") {
            Some(value) => super::secrets::expand_user_path(
                value
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .context("openai_codex auth_file must be a non-empty string")?,
            ),
            None => default_auth_file()?,
        };
        let issuer = match config.get("oauth_issuer") {
            Some(value) => value
                .as_str()
                .context("openai_codex oauth_issuer must be a string")?,
            None => ISSUER,
        };
        Ok(Self {
            path,
            oauth: oauth::OAuthClient::new(issuer)?,
        })
    }

    pub async fn access(&self, rejected_token: Option<String>) -> Result<Access> {
        // A caller's cancellation must not abandon a refresh after the server
        // rotated the token but before we saved it. The bounded transaction
        // retains the OS lock and completes on this worker's runtime.
        let lock = store::lock(&self.path).await?;
        let auth = self.clone();
        let credentials = tokio::spawn(async move {
            let _lock = lock;
            let current = store::read(&auth.path)?.with_context(|| {
                format!(
                    "ChatGPT login required; run `{LOGIN_HINT}` (auth file: {})",
                    auth.path.display()
                )
            })?;
            let rejected = rejected_token.as_deref() == Some(current.access_token.as_str());
            if !rejected && current.expires_at > oauth::now() + 60 {
                return Ok::<_, anyhow::Error>(current);
            }
            let refreshed = auth.oauth.refresh(&current).await?;
            store::write(&auth.path, &refreshed)?;
            Ok(refreshed)
        })
        .await
        .context("ChatGPT credential task failed")??;

        let mut headers = HeaderMap::new();
        let mut bearer = HeaderValue::from_str(&format!("Bearer {}", credentials.access_token))
            .context("invalid ChatGPT access token")?;
        bearer.set_sensitive(true);
        headers.insert(AUTHORIZATION, bearer);
        let mut account =
            HeaderValue::from_str(&credentials.account_id).context("invalid ChatGPT account id")?;
        account.set_sensitive(true);
        headers.insert("chatgpt-account-id", account);
        headers.insert("originator", HeaderValue::from_static("proteus"));
        headers.insert(
            "user-agent",
            HeaderValue::from_static(concat!("proteus/", env!("CARGO_PKG_VERSION"))),
        );
        Ok(Access {
            token: credentials.access_token,
            headers,
        })
    }
}

fn default_auth_file() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is required, or set auth_file explicitly")?;
    Ok(PathBuf::from(home).join(".config/Proteus-agent/secrets/chatgpt.json"))
}

pub(super) fn validate_endpoint(value: &str) -> Result<String> {
    let url = reqwest::Url::parse(value).context("invalid ChatGPT endpoint URL")?;
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!(
            "ChatGPT endpoint requires HTTPS (or loopback HTTP), without credentials, query or fragment"
        );
    }
    Ok(value.trim_end_matches('/').to_owned())
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()?)
}
