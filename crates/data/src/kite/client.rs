//! The HTTP base: allowlisted HTTPS `GET` only, no redirects, no proxy, no automatic retry,
//! a timeout on every call and a cap on every body. Limits and hosts come from
//! `data.toml [kite]`. Rate limiting and retry sit above this layer.

use std::time::Duration;

use bazaar_core::config::KiteSection;
use bazaar_core::secret::Secret;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::{Url, redirect, retry};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::{ApiErrorType, KiteError};

/// What a signed-in call sends. The caller checks token expiry. `Debug` is redacted.
#[derive(Debug, Clone, Copy)]
pub struct KiteCredentials<'a> {
    pub api_key: &'a Secret,
    pub access_token: &'a Secret,
}

#[derive(Debug, Clone, Copy)]
enum Scheme {
    Https,
    /// Plain HTTP to a mock server on 127.0.0.1; does not exist outside this crate's tests.
    #[cfg(test)]
    LoopbackHttp,
}

#[derive(Debug)]
pub struct KiteClient {
    http: reqwest::Client,
    base: Url,
    allowed_hosts: Vec<String>,
    scheme: Scheme,
    max_bytes: u64,
}

#[derive(Deserialize)]
struct Head {
    status: Option<String>,
    error_type: Option<String>,
}

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
}

impl KiteClient {
    /// A production client for `[kite]`. HTTPS only.
    pub fn new(config: &KiteSection) -> Result<Self, KiteError> {
        Self::build(config, Scheme::Https)
    }

    fn build(config: &KiteSection, scheme: Scheme) -> Result<Self, KiteError> {
        if config.connect_timeout_secs == 0 || config.request_timeout_secs == 0 {
            return Err(KiteError::Config("timeouts must be above zero"));
        }
        if config.max_response_bytes == 0 {
            return Err(KiteError::Config("max_response_bytes must be above zero"));
        }
        let version = HeaderValue::from_str(&config.api_version)
            .map_err(|_| KiteError::Config("api_version is not a header value"))?;
        let mut headers = HeaderMap::new();
        headers.insert("X-Kite-Version", version);
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(config.request_timeout_secs.into()))
            .connect_timeout(Duration::from_secs(config.connect_timeout_secs.into()))
            .redirect(redirect::Policy::none())
            .retry(retry::never())
            .no_proxy()
            .referer(false)
            .https_only(matches!(scheme, Scheme::Https))
            .build()
            .map_err(|_| KiteError::Config("http client could not be built"))?;
        let base = Url::parse(&config.api_base).map_err(|_| KiteError::DisallowedTarget)?;
        let client = Self {
            http,
            base,
            allowed_hosts: config.allowed_hosts.clone(),
            scheme,
            max_bytes: config.max_response_bytes,
        };
        client.check_target(&client.base)?;
        if client.base.path() != "/"
            || client.base.query().is_some()
            || client.base.fragment().is_some()
        {
            return Err(KiteError::Config("api_base must end at the host"));
        }
        Ok(client)
    }

    fn check_target(&self, url: &Url) -> Result<(), KiteError> {
        let host = url.host_str().unwrap_or_default();
        let transport_ok = match self.scheme {
            Scheme::Https => url.scheme() == "https" && url.port().is_none(),
            #[cfg(test)]
            Scheme::LoopbackHttp => url.scheme() == "http" && host == "127.0.0.1",
        };
        let ok = transport_ok
            && url.username().is_empty()
            && url.password().is_none()
            && self.allowed_hosts.iter().any(|h| h == host);
        ok.then_some(()).ok_or(KiteError::DisallowedTarget)
    }

    /// `path` is a plain absolute path; parameters go in `query`, which is encoded.
    fn url(&self, path: &str, query: &[(&str, &str)]) -> Result<Url, KiteError> {
        let plain = path.starts_with('/')
            && !path.contains("//")
            && !path.split('/').any(|part| part == "." || part == "..")
            && path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b));
        if !plain {
            return Err(KiteError::InvalidRequest("path is not a plain path"));
        }
        let mut url = self.base.clone();
        url.set_path(path);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        self.check_target(&url)?;
        Ok(url)
    }

    /// `GET` a raw body (the instruments dump is CSV). Only a 200 is data.
    pub async fn get_bytes(
        &self,
        path: &str,
        query: &[(&str, &str)],
        credentials: KiteCredentials<'_>,
    ) -> Result<Vec<u8>, KiteError> {
        let response = self
            .http
            .get(self.url(path, query)?)
            .header(AUTHORIZATION, authorization(credentials)?)
            .send()
            .await
            .map_err(|e| transport_error(&e))?;
        let status = response.status().as_u16();
        let body = self.read_capped(response).await;
        if status == 200 {
            return body;
        }
        // The status decides; an unreadable error body only loses the error type.
        let head = body.ok().and_then(|b| serde_json::from_slice(&b).ok());
        let wire = head.and_then(|h: Head| h.error_type);
        let kind = ApiErrorType::from_wire(wire.as_deref());
        Err(KiteError::from_status(status, kind))
    }

    /// `GET` a JSON endpoint and unwrap Kite's `{"status": "success", "data": ..}` envelope.
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
        credentials: KiteCredentials<'_>,
    ) -> Result<T, KiteError> {
        let body = self.get_bytes(path, query, credentials).await?;
        let head: Head = decode(&body)?;
        if head.status.as_deref() != Some("success") {
            return Err(KiteError::Rejected {
                status: 200,
                kind: ApiErrorType::from_wire(head.error_type.as_deref()),
            });
        }
        decode::<Envelope<T>>(&body).map(|e| e.data)
    }

    /// Read at most `max_bytes`, whether or not the server declares a length.
    async fn read_capped(&self, mut response: reqwest::Response) -> Result<Vec<u8>, KiteError> {
        let limit = self.max_bytes;
        let too_large = KiteError::TooLarge { limit };
        if response.content_length().is_some_and(|n| n > limit) {
            return Err(too_large);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| transport_error(&e))? {
            let total = body.len().saturating_add(chunk.len());
            if u64::try_from(total).map_or(true, |n| n > limit) {
                return Err(too_large);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

fn authorization(credentials: KiteCredentials<'_>) -> Result<HeaderValue, KiteError> {
    let KiteCredentials {
        api_key,
        access_token,
    } = credentials;
    if api_key.is_blank() || access_token.is_blank() {
        return Err(KiteError::LoginRequired);
    }
    let text = format!("token {}:{}", api_key.expose(), access_token.expose());
    let mut value = HeaderValue::from_str(&text)
        .map_err(|_| KiteError::InvalidRequest("credential is not a header value"))?;
    value.set_sensitive(true);
    Ok(value)
}

/// `reqwest` errors carry the URL, so only the category is kept.
fn transport_error(error: &reqwest::Error) -> KiteError {
    if error.is_timeout() {
        KiteError::Timeout
    } else {
        KiteError::Transport
    }
}

fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, KiteError> {
    serde_json::from_slice(body).map_err(|e| KiteError::Decode {
        line: e.line(),
        column: e.column(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
