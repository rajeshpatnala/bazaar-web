//! Typed Kite failures. No variant holds a URL, a header, a body or upstream text, so an
//! error can be logged or shown without leaking a credential or account data.

use thiserror::Error;

/// The `error_type` of a Kite error body, reduced to a closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiErrorType {
    Token,
    Permission,
    User,
    Input,
    Data,
    Network,
    General,
    /// A type this client does not know.
    Other,
    /// No readable error body.
    Missing,
}

impl ApiErrorType {
    pub(super) fn from_wire(name: Option<&str>) -> Self {
        match name {
            Some("TokenException") => Self::Token,
            Some("PermissionException") => Self::Permission,
            Some("UserException") => Self::User,
            Some("InputException") => Self::Input,
            Some("DataException") => Self::Data,
            Some("NetworkException") => Self::Network,
            Some("GeneralException") => Self::General,
            Some(_) => Self::Other,
            None => Self::Missing,
        }
    }
}

/// Why a Kite call gave no data. Callers treat every variant as "no data".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum KiteError {
    #[error("kite client setting is invalid: {0}")]
    Config(&'static str),
    #[error("kite request target is not on the allowlist")]
    DisallowedTarget,
    #[error("kite request is invalid: {0}")]
    InvalidRequest(&'static str),
    /// The broker rejected the session (403 `TokenException`), or there is no credential.
    #[error("kite login required")]
    LoginRequired,
    #[error("kite rate limit reached")]
    RateLimited,
    /// Redirects are never followed.
    #[error("kite answered with a redirect (status {status})")]
    Redirect { status: u16 },
    /// Any other refusal, including a 200 whose envelope does not say success.
    #[error("kite rejected the request (status {status}, {kind:?})")]
    Rejected { status: u16, kind: ApiErrorType },
    #[error("kite server error (status {status}, {kind:?})")]
    Upstream { status: u16, kind: ApiErrorType },
    #[error("kite call timed out")]
    Timeout,
    #[error("kite connection failed")]
    Transport,
    #[error("kite response exceeds {limit} bytes")]
    TooLarge { limit: u64 },
    /// Position only: parser messages can quote values from the body.
    #[error("kite response is not the expected JSON (line {line}, column {column})")]
    Decode { line: usize, column: usize },
}

impl KiteError {
    /// The error for a response whose status is not 200.
    pub(super) fn from_status(status: u16, kind: ApiErrorType) -> Self {
        match (status, kind) {
            (403, ApiErrorType::Token) => Self::LoginRequired,
            (429, _) => Self::RateLimited,
            (300..=399, _) => Self::Redirect { status },
            (500..=599, _) => Self::Upstream { status, kind },
            _ => Self::Rejected { status, kind },
        }
    }
}
