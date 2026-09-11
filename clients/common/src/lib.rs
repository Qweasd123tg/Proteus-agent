use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use url::{Host, Url};

pub mod desktop;
pub mod pending;

/// Per-window session selection is scoped to one exact app-server origin.
pub fn selected_session_storage_key(app_server_origin: &str) -> String {
    format!("proteus.selectedSessionDir:{app_server_origin}")
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SessionCredential {
    pub app_server_origin: String,
    pub token: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CredentialStorageUpdate {
    Keep,
    Replace(SessionCredential),
    Remove,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedCredential {
    pub token: Option<String>,
    pub storage_update: CredentialStorageUpdate,
}

/// Selects a credential for one exact normalized app-server origin.
///
/// A token supplied in the current URL is an explicit new pairing. A stored
/// token is reusable only for the same origin; selecting another server
/// removes it instead of forwarding it to the new recipient.
pub fn resolve_credential(
    app_server_origin: &str,
    query_token: Option<String>,
    stored: Option<SessionCredential>,
) -> ResolvedCredential {
    if let Some(token) = query_token {
        let token = token.trim();
        if token.is_empty() {
            return ResolvedCredential {
                token: None,
                storage_update: CredentialStorageUpdate::Remove,
            };
        }

        let credential = SessionCredential {
            app_server_origin: app_server_origin.to_owned(),
            token: token.to_owned(),
        };
        return ResolvedCredential {
            token: Some(credential.token.clone()),
            storage_update: CredentialStorageUpdate::Replace(credential),
        };
    }

    match stored {
        Some(credential) if credential.app_server_origin == app_server_origin => {
            ResolvedCredential {
                token: Some(credential.token),
                storage_update: CredentialStorageUpdate::Keep,
            }
        }
        Some(_) => ResolvedCredential {
            token: None,
            storage_update: CredentialStorageUpdate::Remove,
        },
        None => ResolvedCredential {
            token: None,
            storage_update: CredentialStorageUpdate::Keep,
        },
    }
}

/// Parses and canonicalizes the local HTTP(S) origin accepted by browser
/// clients. Paths, queries, fragments, credentials and non-loopback hosts are
/// rejected so an app-server or sibling-client endpoint cannot silently turn
/// into an arbitrary credential recipient.
pub fn normalize_local_origin(value: &str) -> Result<String, String> {
    let url = Url::parse(value.trim()).map_err(|error| format!("invalid origin: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("origin scheme must be http or https".to_owned());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("origin must not contain user info".to_owned());
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err("origin must not contain a path, query, or fragment".to_owned());
    }
    let is_loopback = match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => IpAddr::V4(address).is_loopback(),
        Some(Host::Ipv6(address)) => IpAddr::V6(address).is_loopback(),
        None => false,
    };
    if !is_loopback {
        return Err("origin host must be localhost or a loopback IP address".to_owned());
    }

    Ok(url.origin().ascii_serialization())
}

/// Credentials may cross a browser-client link only to the packaged sibling
/// origin. A custom local UI endpoint remains navigable, but must establish
/// its own pairing instead of receiving the current app-server token.
pub fn credential_for_client_link<'a>(
    destination_origin: &str,
    packaged_origin: &str,
    token: Option<&'a str>,
) -> Option<&'a str> {
    (destination_origin == packaged_origin)
        .then_some(token)
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_credential_is_reused_only_for_its_exact_origin() {
        let stored = SessionCredential {
            app_server_origin: "http://127.0.0.1:8787".to_owned(),
            token: "token-a".to_owned(),
        };

        let same = resolve_credential("http://127.0.0.1:8787", None, Some(stored.clone()));
        assert_eq!(same.token.as_deref(), Some("token-a"));
        assert_eq!(same.storage_update, CredentialStorageUpdate::Keep);

        let changed = resolve_credential("http://127.0.0.1:9000", None, Some(stored));
        assert_eq!(changed.token, None);
        assert_eq!(changed.storage_update, CredentialStorageUpdate::Remove);
    }

    #[test]
    fn query_token_creates_a_new_origin_pairing() {
        let resolved = resolve_credential(
            "http://127.0.0.1:9000",
            Some(" token-b ".to_owned()),
            Some(SessionCredential {
                app_server_origin: "http://127.0.0.1:8787".to_owned(),
                token: "token-a".to_owned(),
            }),
        );

        assert_eq!(resolved.token.as_deref(), Some("token-b"));
        assert_eq!(
            resolved.storage_update,
            CredentialStorageUpdate::Replace(SessionCredential {
                app_server_origin: "http://127.0.0.1:9000".to_owned(),
                token: "token-b".to_owned(),
            })
        );
    }

    #[test]
    fn local_origin_parser_rejects_non_origin_and_remote_urls() {
        assert_eq!(
            normalize_local_origin(" http://localhost:8787/ ").unwrap(),
            "http://localhost:8787"
        );
        assert_eq!(
            normalize_local_origin("https://[::1]:9443").unwrap(),
            "https://[::1]:9443"
        );

        for invalid in [
            "javascript:alert(1)",
            "http://example.com:8787",
            "http://127.0.0.1:8787/api",
            "http://user:secret@127.0.0.1:8787",
            "http://127.0.0.1:8787/?token=secret",
        ] {
            assert!(
                normalize_local_origin(invalid).is_err(),
                "unexpected valid origin: {invalid}"
            );
        }
    }

    #[test]
    fn credential_is_not_forwarded_to_a_custom_client_origin() {
        assert_eq!(
            credential_for_client_link(
                "http://127.0.0.1:1421",
                "http://127.0.0.1:1421",
                Some("token-a")
            ),
            Some("token-a")
        );
        assert_eq!(
            credential_for_client_link(
                "http://127.0.0.1:9001",
                "http://127.0.0.1:1421",
                Some("token-a")
            ),
            None
        );
    }
}

pub mod execution;

pub mod run_options;
