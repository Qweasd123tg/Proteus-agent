use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use serde_json::Value;

use super::{CLIENT_ID, LOGIN_HINT, http_client, store::Credentials, validate_endpoint};

#[derive(Clone, Debug)]
pub(super) struct OAuthClient {
    pub issuer: String,
    pub http: reqwest::Client,
}

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<u64>,
}

pub(super) struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

impl Pkce {
    pub fn new() -> Self {
        let verifier = random_secret();
        let challenge = URL_SAFE_NO_PAD.encode(ring::digest::digest(
            &ring::digest::SHA256,
            verifier.as_bytes(),
        ));
        Self {
            verifier,
            challenge,
            state: random_secret(),
        }
    }
}

fn random_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl OAuthClient {
    pub fn new(issuer: &str) -> Result<Self> {
        Ok(Self {
            issuer: validate_endpoint(issuer)?,
            http: http_client()?,
        })
    }

    pub fn authorize_url(&self, redirect: &str, pkce: &Pkce) -> Result<reqwest::Url> {
        let mut url = reqwest::Url::parse(&format!("{}/oauth/authorize", self.issuer))?;
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", redirect),
            ("scope", "openid profile email offline_access"),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("state", &pkce.state),
            ("originator", "proteus"),
        ]);
        Ok(url)
    }

    pub async fn exchange(
        &self,
        code: &str,
        redirect: &str,
        verifier: &str,
    ) -> Result<Credentials> {
        self.token(
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect),
                ("client_id", CLIENT_ID),
                ("code_verifier", verifier),
            ],
            None,
        )
        .await
    }

    pub async fn refresh(&self, old: &Credentials) -> Result<Credentials> {
        self.token(
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &old.refresh_token),
                ("client_id", CLIENT_ID),
            ],
            Some(old),
        )
        .await
    }

    async fn token(
        &self,
        form: &[(&str, &str)],
        previous: Option<&Credentials>,
    ) -> Result<Credentials> {
        // No blind retries: the server may already have rotated the token.
        let response = self
            .http
            .post(format!("{}/oauth/token", self.issuer))
            .form(form)
            .send()
            .await
            .context("ChatGPT token request failed")?;
        if !response.status().is_success() {
            bail!(
                "ChatGPT token request returned {}; run `{LOGIN_HINT}` again",
                response.status()
            );
        }
        let tokens: Tokens = response
            .json()
            .await
            .context("invalid ChatGPT token response")?;
        credentials(tokens, previous)
    }
}

fn claims(token: &str) -> Option<Value> {
    let mut parts = token.split('.');
    parts.next()?;
    let payload = parts.next()?;
    parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()
}

fn account_id(claims: &Value) -> Option<&str> {
    claims
        .get("chatgpt_account_id")
        .or_else(|| {
            claims
                .get("https://api.openai.com/auth")?
                .get("chatgpt_account_id")
        })
        .or_else(|| claims.get("organizations")?.get(0)?.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
}

fn credentials(tokens: Tokens, previous: Option<&Credentials>) -> Result<Credentials> {
    // Claims are metadata from the token endpoint, not a local proof of identity.
    let id_claims = tokens.id_token.as_deref().and_then(claims);
    let access_claims = claims(&tokens.access_token);
    let account = id_claims
        .as_ref()
        .and_then(account_id)
        .or_else(|| access_claims.as_ref().and_then(account_id))
        .or_else(|| previous.map(|old| old.account_id.as_str()))
        .context("ChatGPT token response has no account id")?
        .to_owned();
    if previous.is_some_and(|old| old.account_id != account) {
        bail!("ChatGPT account changed during refresh; sign in again");
    }
    let expires_at = tokens
        .expires_in
        .map(|seconds| now().saturating_add(seconds))
        .or_else(|| access_claims.as_ref()?.get("exp")?.as_u64())
        .unwrap_or_else(|| now() + 3600);
    let credential = Credentials {
        access_token: tokens.access_token,
        refresh_token: tokens
            .refresh_token
            .or_else(|| previous.map(|old| old.refresh_token.clone()))
            .context("ChatGPT token response has no refresh token")?,
        account_id: account,
        expires_at,
    };
    credential.validate()?;
    Ok(credential)
}
