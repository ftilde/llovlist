use std::fmt;
use std::str::FromStr;

use axum::extract::{FromRef, FromRequestParts};
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const TOKEN_BYTES: usize = 16;
const HASH_BYTES: usize = 32;

pub struct Token([u8; TOKEN_BYTES]);

impl Token {
    pub fn generate() -> Self {
        let mut bytes = [0; TOKEN_BYTES];
        getrandom::fill(&mut bytes).expect("the OS refused to provide randomness");
        Self(bytes)
    }

    pub fn hash(&self) -> TokenHash {
        TokenHash(Sha256::digest(self.0).into())
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenHash([u8; HASH_BYTES]);

impl TokenHash {
    pub fn accepts(&self, presented: &str) -> bool {
        let mut token = [0; TOKEN_BYTES];
        if hex::decode_to_slice(presented, &mut token).is_err() {
            return false;
        }
        let hash: [u8; HASH_BYTES] = Sha256::digest(token).into();
        hash.ct_eq(&self.0).into()
    }
}

impl fmt::Display for TokenHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl FromStr for TokenHash {
    type Err = hex::FromHexError;

    fn from_str(raw: &str) -> Result<Self, hex::FromHexError> {
        let mut bytes = [0; HASH_BYTES];
        hex::decode_to_slice(raw, &mut bytes)?;
        Ok(Self(bytes))
    }
}

pub struct Authenticated;

impl<S> FromRequestParts<S> for Authenticated
where
    S: Send + Sync,
    TokenHash: FromRef<S>,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, StatusCode> {
        let presented = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(StatusCode::UNAUTHORIZED)?;

        if TokenHash::from_ref(state).accepts(presented) {
            Ok(Self)
        } else {
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_accepts_its_own_token_and_nothing_else() {
        let token = Token::generate();
        let hash = token.hash();

        assert!(hash.accepts(&token.to_string()));
        for presented in ["", "not hex", "zz", &Token::generate().to_string(), &"a".repeat(1000)] {
            assert!(!hash.accepts(presented), "accepted {presented:?}");
        }
    }
}
