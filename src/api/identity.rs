use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClientIdentity {
    User {
        subject: String,
        tenant_id: Option<String>,
    },
    ApiKey {
        key_id: String,
    },
    Ip {
        address: IpAddr,
    },
}

fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '%' => out.push_str("%25"),
            ':' => out.push_str("%3A"),
            _ => out.push(ch),
        }
    }
    out
}

impl ClientIdentity {
    pub fn rate_limit_key(&self) -> String {
        match self {
            ClientIdentity::User {
                subject,
                tenant_id: Some(tenant),
            } => format!(
                "tenant:{}:user:{}",
                encode_component(tenant),
                encode_component(subject)
            ),
            ClientIdentity::User {
                subject,
                tenant_id: None,
            } => format!("user:{}", encode_component(subject)),
            ClientIdentity::ApiKey { key_id } => format!("key:{}", encode_component(key_id)),
            ClientIdentity::Ip { address } => format!("ip:{}", address),
        }
    }

    pub fn concurrency_key(&self) -> String {
        self.rate_limit_key()
    }

    pub fn is_user(&self) -> bool {
        matches!(self, ClientIdentity::User { .. })
    }
}

pub fn normalize_identity_key(identity_key: &str, configured_api_keys: &[String]) -> String {
    if let Some(key_val) = identity_key.strip_prefix("key:") {
        // Modern 64-character SHA-256 hex string: already normalized
        if key_val.len() == 64 && key_val.chars().all(|c| c.is_ascii_hexdigit()) {
            return identity_key.to_string();
        }

        // Check if key_val matches the legacy SHA-1 of any configured key
        for configured in configured_api_keys {
            let mut s1 = sha1_smol::Sha1::new();
            s1.update(configured.as_bytes());
            if s1.digest().to_string() == key_val {
                use sha2::{Digest, Sha256};
                let mut s2 = Sha256::new();
                s2.update(configured.as_bytes());
                return format!("key:{:x}", s2.finalize());
            }
        }

        // Check if key_val matches any configured key directly (legacy raw key)
        for configured in configured_api_keys {
            if configured == key_val {
                use sha2::{Digest, Sha256};
                let mut s2 = Sha256::new();
                s2.update(configured.as_bytes());
                return format!("key:{:x}", s2.finalize());
            }
        }

        // Unrecognized unhashed key: hash with SHA-256 to ensure consistent length and format
        use sha2::{Digest, Sha256};
        let mut s2 = Sha256::new();
        s2.update(key_val.as_bytes());
        format!("key:{:x}", s2.finalize())
    } else {
        identity_key.to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserAssertionClaims {
    pub sub: String,
    #[serde(default)]
    pub iss: Option<String>,
    #[serde(default)]
    pub aud: Option<String>,
    pub exp: u64,
    #[serde(default)]
    pub nbf: Option<u64>,
    #[serde(default)]
    pub iat: Option<u64>,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("Missing identity assertion")]
    MissingAssertion,
    #[error("Invalid JWT assertion: {0}")]
    InvalidJwt(String),
    #[error("JWT secret not configured")]
    SecretNotConfigured,
    #[error("Audience mismatch")]
    AudienceMismatch,
    #[error("Issuer mismatch")]
    IssuerMismatch,
    #[error("Invalid identity header: {0}")]
    InvalidHeader(String),
}

pub fn verify_jwt_assertion(
    token: &str,
    secret: &str,
    expected_issuer: Option<&str>,
    expected_audience: Option<&str>,
) -> Result<ClientIdentity, IdentityError> {
    use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};

    let mut validation = Validation::new(Algorithm::HS256);
    if let Some(aud) = expected_audience {
        validation.set_audience(&[aud]);
    } else {
        validation.validate_aud = false;
    }
    if let Some(iss) = expected_issuer {
        validation.set_issuer(&[iss]);
    }

    let token_data = decode::<UserAssertionClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|e| IdentityError::InvalidJwt(e.to_string()))?;

    let subject = token_data.claims.sub;
    if subject.trim().is_empty() {
        return Err(IdentityError::InvalidJwt(
            "Subject cannot be empty or whitespace".into(),
        ));
    }

    Ok(ClientIdentity::User {
        subject,
        tenant_id: token_data.claims.tenant_id,
    })
}

pub fn extract_ip(
    headers: &axum::http::HeaderMap,
    connect_info: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    trusted_proxies: &[IpAddr],
) -> IpAddr {
    match connect_info {
        Some(conn) => {
            let peer_ip = conn.0.ip();
            if trusted_proxies.contains(&peer_ip) {
                headers
                    .get("x-forwarded-for")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .and_then(|s| s.trim().parse::<IpAddr>().ok())
                    .or_else(|| {
                        headers
                            .get("x-real-ip")
                            .and_then(|h| h.to_str().ok())
                            .and_then(|s| s.trim().parse::<IpAddr>().ok())
                    })
                    .unwrap_or(peer_ip)
            } else {
                peer_ip
            }
        }
        None => headers
            .get("x-forwarded-for")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.split(',').next())
            .and_then(|s| s.trim().parse::<IpAddr>().ok())
            .or_else(|| {
                headers
                    .get("x-real-ip")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.trim().parse::<IpAddr>().ok())
            })
            .unwrap_or_else(|| "127.0.0.1".parse().unwrap()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ConnectInfo;
    use axum::http::HeaderMap;
    use jsonwebtoken::{encode, EncodingKey, Header};

    #[test]
    fn test_identity_keys() {
        let user1 = ClientIdentity::User {
            subject: "user-42".into(),
            tenant_id: None,
        };
        assert_eq!(user1.rate_limit_key(), "user:user-42");
        assert_eq!(user1.concurrency_key(), "user:user-42");
        assert!(user1.is_user());

        let user2 = ClientIdentity::User {
            subject: "user-42".into(),
            tenant_id: Some("tenant-a".into()),
        };
        assert_eq!(user2.rate_limit_key(), "tenant:tenant-a:user:user-42");

        let key = ClientIdentity::ApiKey {
            key_id: "sec_abc123".into(),
        };
        assert_eq!(key.rate_limit_key(), "key:sec_abc123");
        assert!(!key.is_user());

        let ip = ClientIdentity::Ip {
            address: "127.0.0.1".parse().unwrap(),
        };
        assert_eq!(ip.rate_limit_key(), "ip:127.0.0.1");
        assert!(!ip.is_user());
    }

    #[test]
    fn test_encode_component_utf8() {
        assert_eq!(encode_component("user:123"), "user%3A123");
        assert_eq!(encode_component("100%"), "100%25");
        assert_eq!(encode_component("🦀_user"), "🦀_user");
        assert_eq!(encode_component("用户:1"), "用户%3A1");
    }

    #[test]
    fn test_colon_in_tenant_or_subject_does_not_collide() {
        let u1 = ClientIdentity::User {
            subject: "user2".into(),
            tenant_id: Some("tenant:1".into()),
        };
        let u2 = ClientIdentity::User {
            subject: "1:user:user2".into(),
            tenant_id: Some("tenant".into()),
        };
        assert_ne!(u1.rate_limit_key(), u2.rate_limit_key());
        assert_ne!(u1.concurrency_key(), u2.concurrency_key());
    }

    #[test]
    fn test_normalize_identity_key_legacy_upgrade() {
        let configured = vec!["client_key".to_string(), "admin_key".to_string()];
        
        // Legacy raw key
        let norm_raw = normalize_identity_key("key:client_key", &configured);
        assert_eq!(norm_raw, "key:d9ee725310e983561b0447bc0f5cffc57161c33f3a64051568e24fc3fc8a5d18");

        // Legacy SHA-1 hash of client_key
        let norm_sha1 = normalize_identity_key("key:ee369845b98b65e65abb99e72a3bec006a78d3e8", &configured);
        assert_eq!(norm_sha1, "key:d9ee725310e983561b0447bc0f5cffc57161c33f3a64051568e24fc3fc8a5d18");

        // Modern SHA-256 hash stays identical
        let norm_sha256 = normalize_identity_key("key:d9ee725310e983561b0447bc0f5cffc57161c33f3a64051568e24fc3fc8a5d18", &configured);
        assert_eq!(norm_sha256, "key:d9ee725310e983561b0447bc0f5cffc57161c33f3a64051568e24fc3fc8a5d18");

        // User or IP keys are untouched
        assert_eq!(normalize_identity_key("user:123", &configured), "user:123");
        assert_eq!(normalize_identity_key("ip:127.0.0.1", &configured), "ip:127.0.0.1");
    }

    #[test]
    fn test_jwt_verification_valid() {
        let secret = "test-secret-key-1234567890";
        let claims = UserAssertionClaims {
            sub: "usr-100".into(),
            iss: Some("test-issuer".into()),
            aud: Some("otter".into()),
            exp: 9999999999,
            nbf: None,
            iat: None,
            tenant_id: Some("tenant-xyz".into()),
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let res = verify_jwt_assertion(&token, secret, Some("test-issuer"), Some("otter"));
        assert!(res.is_ok());
        let identity = res.unwrap();
        assert_eq!(
            identity,
            ClientIdentity::User {
                subject: "usr-100".into(),
                tenant_id: Some("tenant-xyz".into()),
            }
        );
    }

    #[test]
    fn test_jwt_verification_empty_subject_rejected() {
        let secret = "test-secret-key-1234567890";
        let claims = UserAssertionClaims {
            sub: "   ".into(),
            iss: None,
            aud: None,
            exp: 9999999999,
            nbf: None,
            iat: None,
            tenant_id: None,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let res = verify_jwt_assertion(&token, secret, None, None);
        assert!(matches!(res, Err(IdentityError::InvalidJwt(_))));
    }

    #[test]
    fn test_jwt_verification_expired() {
        let secret = "test-secret-key-1234567890";
        let claims = UserAssertionClaims {
            sub: "usr-100".into(),
            iss: None,
            aud: None,
            exp: 1000, // Past expiration
            nbf: None,
            iat: None,
            tenant_id: None,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();

        let res = verify_jwt_assertion(&token, secret, None, None);
        assert!(matches!(res, Err(IdentityError::InvalidJwt(_))));
    }

    #[test]
    fn test_jwt_verification_wrong_secret() {
        let claims = UserAssertionClaims {
            sub: "usr-100".into(),
            iss: None,
            aud: None,
            exp: 9999999999,
            nbf: None,
            iat: None,
            tenant_id: None,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(b"secret-1"),
        )
        .unwrap();

        let res = verify_jwt_assertion(&token, "secret-2", None, None);
        assert!(matches!(res, Err(IdentityError::InvalidJwt(_))));
    }
    #[test]
    fn test_extract_ip_untrusted_proxy_ignored() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.195".parse().unwrap());
        let connect_info = Some(ConnectInfo("198.51.100.1:12345".parse().unwrap()));
        let trusted_proxies = vec!["10.0.0.1".parse().unwrap()];

        let ip = extract_ip(&headers, connect_info, &trusted_proxies);
        assert_eq!(ip, "198.51.100.1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_extract_ip_trusted_proxy_honored() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.195, 10.0.0.1".parse().unwrap());
        let connect_info = Some(ConnectInfo("10.0.0.1:12345".parse().unwrap()));
        let trusted_proxies = vec!["10.0.0.1".parse().unwrap()];

        let ip = extract_ip(&headers, connect_info, &trusted_proxies);
        assert_eq!(ip, "203.0.113.195".parse::<IpAddr>().unwrap());
    }
}
