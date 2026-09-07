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

impl ClientIdentity {
    pub fn rate_limit_key(&self) -> String {
        match self {
            ClientIdentity::User {
                subject,
                tenant_id: Some(tenant),
            } => format!("tenant:{}:user:{}", tenant, subject),
            ClientIdentity::User {
                subject,
                tenant_id: None,
            } => format!("user:{}", subject),
            ClientIdentity::ApiKey { key_id } => format!("key:{}", key_id),
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

    Ok(ClientIdentity::User {
        subject: token_data.claims.sub,
        tenant_id: token_data.claims.tenant_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
