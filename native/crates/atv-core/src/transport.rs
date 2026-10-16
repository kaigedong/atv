use reqwest::blocking::Client;
use std::{
    fmt,
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use thiserror::Error;
use url::Url;
#[derive(Debug, Error)]
pub enum HttpError {
    #[error("request cancelled")]
    Cancelled,
    #[error("only HTTPS network resources are supported")]
    Insecure,
    #[error("network request failed")]
    Network,
    #[error("network request timed out")]
    Timeout,
    #[error("upstream returned HTTP {0}")]
    Status(u16),
    #[error("network response exceeded its size limit")]
    TooLarge,
}
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release)
    }
    pub fn check(&self) -> Result<(), HttpError> {
        if self.0.load(Ordering::Acquire) {
            Err(HttpError::Cancelled)
        } else {
            Ok(())
        }
    }
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    /// Share the same flag with protocol resolvers without creating a second cancellation source.
    pub fn flag(&self) -> Arc<AtomicBool> {
        self.0.clone()
    }
}
/// Downloaded bytes plus the final HTTPS response URL used as the base for
/// relative playlist references. Debug never formats signed resources or bytes.
pub struct Resource {
    pub bytes: Vec<u8>,
    pub final_url: Url,
}
impl fmt::Debug for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Resource")
            .field("bytes", &self.bytes.len())
            .field("final_url", &"[redacted]")
            .finish()
    }
}
#[derive(Clone)]
pub struct HttpClient {
    client: Client,
}
impl HttpClient {
    pub fn new(user_agent: &str) -> Result<Self, HttpError> {
        Ok(Self {
            client: Client::builder()
                .https_only(true)
                .user_agent(user_agent)
                .connect_timeout(Duration::from_secs(6))
                .timeout(Duration::from_secs(12))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.url().scheme() != "https" || attempt.previous().len() >= 3 {
                        attempt.stop()
                    } else {
                        attempt.follow()
                    }
                }))
                .build()
                .map_err(|_| HttpError::Network)?,
        })
    }
    pub fn get(
        &self,
        url: &Url,
        limit: usize,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, HttpError> {
        self.get_range(url, limit, None, cancel)
    }
    /// Follow authorized HTTPS media redirects while retaining the actual base
    /// URL. Callers parsing relative links must use `resource.final_url`.
    pub fn get_resource(
        &self,
        url: &Url,
        limit: usize,
        cancel: &Cancellation,
    ) -> Result<Resource, HttpError> {
        self.get_range_resource(url, limit, None, cancel)
    }
    pub fn get_range(
        &self,
        url: &Url,
        limit: usize,
        range: Option<(u64, u64)>,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, HttpError> {
        self.get_range_resource(url, limit, range, cancel)
            .map(|resource| resource.bytes)
    }
    fn get_range_resource(
        &self,
        url: &Url,
        limit: usize,
        range: Option<(u64, u64)>,
        cancel: &Cancellation,
    ) -> Result<Resource, HttpError> {
        cancel.check()?;
        if url.scheme() != "https" {
            return Err(HttpError::Insecure);
        }
        let mut request = self.client.get(url.clone());
        if let Some((offset, length)) = range {
            let end = offset
                .checked_add(length)
                .and_then(|n| n.checked_sub(1))
                .filter(|_| length > 0 && length <= limit as u64)
                .ok_or(HttpError::TooLarge)?;
            request = request.header(reqwest::header::RANGE, format!("bytes={offset}-{end}"));
        }
        let mut response = request.send().map_err(|e| {
            if e.is_timeout() {
                HttpError::Timeout
            } else {
                HttpError::Network
            }
        })?;
        // Redirects never weaken TLS, even when the upstream supplies an HTTP location.
        if response.url().scheme() != "https" {
            return Err(HttpError::Insecure);
        }
        if !response.status().is_success() {
            return Err(HttpError::Status(response.status().as_u16()));
        }
        if range.is_some() && response.status().as_u16() != 206 {
            return Err(HttpError::Status(response.status().as_u16()));
        }
        if response.content_length().is_some_and(|n| n > limit as u64) {
            return Err(HttpError::TooLarge);
        }
        let final_url = response.url().clone();
        // The advertised size is already bounded above. Avoid geometric
        // growth (and retained spare capacity) for ordinary sized responses.
        let mut out = Vec::with_capacity(response.content_length().unwrap_or(0) as usize);
        let mut buffer = [0; 32768];
        loop {
            cancel.check()?;
            let n = response.read(&mut buffer).map_err(|_| HttpError::Network)?;
            if n == 0 {
                break;
            }
            if out.len().saturating_add(n) > limit {
                return Err(HttpError::TooLarge);
            }
            out.extend_from_slice(&buffer[..n]);
        }
        cancel.check()?;
        if range.is_some_and(|(_, length)| out.len() as u64 != length) {
            return Err(HttpError::Network);
        }
        Ok(Resource {
            bytes: out,
            final_url,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resource_debug_omits_signed_url_and_payload() {
        let resource = Resource {
            bytes: b"fixture-sensitive-payload".to_vec(),
            final_url: Url::parse(
                "https://fixture.invalid/newdir/live.m3u8?token=fixture-sensitive-token",
            )
            .unwrap(),
        };
        let debug = format!("{resource:?}");
        assert!(debug.contains("bytes: 25"));
        assert!(!debug.contains("fixture-sensitive-token"));
        assert!(!debug.contains("fixture-sensitive-payload"));
        assert!(!debug.contains("https://"));
    }
}
