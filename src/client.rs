//! A small signed blocking client for the OCI REST API.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::Profile;
use crate::error::{ApiError, Error, Result};
use crate::signer::{Request, Signer};

const DEFAULT_USER_AGENT: &str = concat!("oci-request-signer/", env!("CARGO_PKG_VERSION"));
/// The domain most OCI services answer on: `<service>.<region>.oraclecloud.com`
/// (`iaas`, `identity`, `telemetry`, `objectstorage`, ...). The default.
pub const DEFAULT_DOMAIN: &str = "oraclecloud.com";

/// The domain newer OCI services answer on only: `<service>.<region>.oci.oraclecloud.com`
/// (`usageapi`, `organizations`, `limits`, `usage` for Budgets, `network-load-balancer-api`, ...).
/// Checked 10 November 2026; which service lives where is the caller's knowledge, not this crate's.
pub const OCI_DOMAIN: &str = "oci.oraclecloud.com";

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
/// The longest a throttled request waits before trying again, whatever `Retry-After` says.
const MAX_THROTTLE_WAIT: Duration = Duration::from_secs(60);

/// An HTTP method the client can send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Method {
    /// `GET`
    Get,
    /// `POST` — always signed with body headers, even with no body.
    Post,
    /// `PUT` — always signed with body headers, even with no body.
    Put,
    /// `DELETE`
    Delete,
}

impl Method {
    /// Lower-case name, as it goes into the signature's `(request-target)`.
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "get",
            Method::Post => "post",
            Method::Put => "put",
            Method::Delete => "delete",
        }
    }

    /// OCI signs `content-length`, `content-type` and `x-content-sha256` for these methods
    /// whether or not there is a body.
    fn has_body(self) -> bool {
        matches!(self, Method::Post | Method::Put)
    }
}

/// A successful (2xx) response: status, headers and the raw body.
#[derive(Debug, Clone)]
pub struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    service: String,
}

impl Response {
    /// HTTP status code, e.g. `200` or `204`.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The first value of header `name` (case-insensitive), e.g. `opc-next-page` or `etag`.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// `opc-request-id`, which Oracle support asks for.
    pub fn request_id(&self) -> Option<&str> {
        self.header("opc-request-id")
    }

    /// The raw body; empty for `204 No Content`.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// The body as UTF-8 text.
    pub fn text(&self) -> Result<&str> {
        std::str::from_utf8(&self.body)
            .map_err(|e| Error::Transport(format!("non-UTF-8 response from {}: {e}", self.service)))
    }

    /// The body parsed as JSON.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(|e| {
            Error::Transport(format!("unexpected response from {}: {e}", self.service))
        })
    }
}

/// Configures a [`Client`]. Start with [`Client::builder`].
#[derive(Debug, Clone)]
#[must_use]
pub struct ClientBuilder<'a> {
    profile: &'a Profile,
    user_agent: String,
    endpoint: Option<String>,
    region: Option<String>,
    domain: String,
    connect_timeout: Duration,
    total_timeout: Duration,
    retry_throttled: u32,
}

impl ClientBuilder<'_> {
    /// The `User-Agent` header; defaults to `oci-request-signer/<version>`.
    /// Name your tool here, e.g. `concat!("my-tool/", env!("CARGO_PKG_VERSION"))`.
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Send every request to this base URL (e.g. `http://127.0.0.1:8080`) instead of
    /// `https://<service>.<region>.<domain>`. For tests and demos against a local mock.
    ///
    /// `{region}` and `{service}` in it are replaced per request, so a mock can tell regions and
    /// services apart: `http://127.0.0.1:8080/{region}` sends a `ListInstances` in Frankfurt to
    /// `http://127.0.0.1:8080/eu-frankfurt-1/20160918/instances`. The signature still covers the
    /// path without that prefix — fine for a mock, which does not check it.
    ///
    /// `None` and the empty string leave the real endpoints in place, so an environment
    /// variable can be passed straight in: `.endpoint(std::env::var("MY_TOOL_ENDPOINT").ok())`.
    pub fn endpoint(mut self, endpoint: impl Into<Option<String>>) -> Self {
        self.endpoint = endpoint
            .into()
            .filter(|v| !v.is_empty())
            .map(|v| v.trim_end_matches('/').to_owned());
        self
    }

    /// Talk to `region` (e.g. `us-phoenix-1`) instead of the profile's region.
    pub fn region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// The domain after `<service>.<region>.`; defaults to [`DEFAULT_DOMAIN`]. Use
    /// [`Client::on_domain`] to reach [`OCI_DOMAIN`] services from the same client, or pass
    /// another realm's domain here.
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = domain.into();
        self
    }

    /// Connection timeout and the timeout for a whole request; default 30 s and 120 s.
    pub fn timeouts(mut self, connect: Duration, total: Duration) -> Self {
        self.connect_timeout = connect;
        self.total_timeout = total;
        self
    }

    /// Retry a request answered `429 TooManyRequests` up to `times` times, waiting what
    /// `Retry-After` says (at most 60 s) or 1, 2, 4… s. Off (0) by default: a tool that loops on
    /// its own — like a capacity hunter — should see the 429. 5xx are never retried here: a
    /// `POST` may have been carried out, so that decision is the caller's.
    pub fn retry_throttled(mut self, times: u32) -> Self {
        self.retry_throttled = times;
        self
    }

    /// Load the signing key and build the client.
    pub fn build(self) -> Result<Client> {
        let agent = ureq::Agent::config_builder()
            // We read OCI's error body ourselves.
            .http_status_as_error(false)
            .timeout_connect(Some(self.connect_timeout))
            .timeout_global(Some(self.total_timeout))
            .user_agent(&self.user_agent)
            .build()
            .into();
        Ok(Client {
            signer: Arc::new(Signer::from_profile(self.profile)?),
            region: self.region.unwrap_or_else(|| self.profile.region.clone()),
            domain: self.domain,
            tenancy: self.profile.tenancy.clone(),
            agent,
            endpoint: self.endpoint,
            retry_throttled: self.retry_throttled,
        })
    }
}

/// A signed connection to one region. Cheap to clone; clones share the key and connection pool.
#[derive(Clone)]
pub struct Client {
    signer: Arc<Signer>,
    region: String,
    domain: String,
    tenancy: String,
    agent: ureq::Agent,
    endpoint: Option<String>,
    retry_throttled: u32,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("region", &self.region)
            .field("domain", &self.domain)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// A client with default settings for the profile's tenancy and region.
    pub fn new(profile: &Profile) -> Result<Self> {
        Self::builder(profile).build()
    }

    /// Configure a client for `profile`.
    pub fn builder(profile: &Profile) -> ClientBuilder<'_> {
        ClientBuilder {
            profile,
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            endpoint: None,
            region: None,
            domain: DEFAULT_DOMAIN.to_owned(),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            total_timeout: DEFAULT_TOTAL_TIMEOUT,
            retry_throttled: 0,
        }
    }

    /// The same client pointed at another region — for walking every subscribed region
    /// without reloading the key.
    pub fn in_region(&self, region: impl Into<String>) -> Self {
        Self {
            region: region.into(),
            ..self.clone()
        }
    }

    /// The same client on another domain, e.g. [`OCI_DOMAIN`] for the Usage API:
    /// `client.on_domain(OCI_DOMAIN).post("usageapi", "/20200107/usage", &query, &[])`.
    pub fn on_domain(&self, domain: impl Into<String>) -> Self {
        Self {
            domain: domain.into(),
            ..self.clone()
        }
    }

    /// The region requests go to.
    pub fn region(&self) -> &str {
        &self.region
    }

    /// The domain requests go to, [`DEFAULT_DOMAIN`] unless set.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// The host a request to `service` goes to: `<service>.<region>.<domain>`.
    /// An [endpoint](ClientBuilder::endpoint) override takes precedence when sending.
    pub fn host(&self, service: &str) -> String {
        format!("{service}.{}.{}", self.region, self.domain)
    }

    /// The tenancy OCID, which is also the root compartment.
    pub fn tenancy(&self) -> &str {
        &self.tenancy
    }

    /// `GET` a JSON resource from `service` (`iaas`, `identity`, `telemetry`, ...).
    /// `path_and_query` starts with `/` and has its query values [`encode`](crate::encode)d.
    pub fn get<T: DeserializeOwned>(&self, service: &str, path_and_query: &str) -> Result<T> {
        self.request(Method::Get, service, path_and_query, None, &[])?
            .json()
    }

    /// `POST` a JSON body to `service` and parse the JSON answer.
    pub fn post<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        service: &str,
        path: &str,
        body: &B,
        extra_headers: &[(&str, &str)],
    ) -> Result<T> {
        let body = to_json(body)?;
        self.request(Method::Post, service, path, Some(&body), extra_headers)?
            .json()
    }

    /// `PUT` a JSON body to `service` and parse the JSON answer.
    pub fn put<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        service: &str,
        path: &str,
        body: &B,
        extra_headers: &[(&str, &str)],
    ) -> Result<T> {
        let body = to_json(body)?;
        self.request(Method::Put, service, path, Some(&body), extra_headers)?
            .json()
    }

    /// `DELETE` a resource. OCI usually answers `204 No Content`.
    pub fn delete(
        &self,
        service: &str,
        path: &str,
        extra_headers: &[(&str, &str)],
    ) -> Result<Response> {
        self.request(Method::Delete, service, path, None, extra_headers)
    }

    /// Send a signed request and return the response as is. Non-2xx answers become
    /// [`Error::Api`]. With [`ClientBuilder::retry_throttled`], a `429 TooManyRequests` is
    /// retried after the `Retry-After` the service asks for (or 1, 2, 4… s), re-signed each time.
    ///
    /// `body` is sent as `application/json`. For `POST` and `PUT` without a body an empty one
    /// is signed, as OCI requires (e.g. `POST /instances/{id}?action=START`).
    pub fn request(
        &self,
        method: Method,
        service: &str,
        path_and_query: &str,
        body: Option<&[u8]>,
        extra_headers: &[(&str, &str)],
    ) -> Result<Response> {
        let mut attempt = 0;
        loop {
            match self.send(method, service, path_and_query, body, extra_headers) {
                Err(Error::Api(e)) if e.status == 429 && attempt < self.retry_throttled => {
                    let wait = e
                        .retry_after
                        .unwrap_or_else(|| Duration::from_secs(1 << attempt.min(5)))
                        .min(MAX_THROTTLE_WAIT);
                    std::thread::sleep(wait);
                    attempt += 1;
                }
                other => return other,
            }
        }
    }

    /// Every item of a list, following `opc-next-page`. The answer may be a JSON array
    /// (`ListInstances`) or `{"items": [...]}` (`ListSubscriptions`, network load balancers).
    /// `path_and_query` has its query values [`encode`](crate::encode)d.
    pub fn list<T: DeserializeOwned>(&self, service: &str, path_and_query: &str) -> Result<Vec<T>> {
        let mut all = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let url = match &page {
                Some(p) => {
                    let sep = if path_and_query.contains('?') {
                        '&'
                    } else {
                        '?'
                    };
                    format!("{path_and_query}{sep}page={}", crate::encode(p))
                }
                None => path_and_query.to_owned(),
            };
            let resp = self.request(Method::Get, service, &url, None, &[])?;
            let v: serde_json::Value = resp.json()?;
            let items = match v {
                serde_json::Value::Array(_) => v,
                mut obj => obj
                    .get_mut("items")
                    .map(serde_json::Value::take)
                    .unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
            };
            all.extend(serde_json::from_value::<Vec<T>>(items).map_err(|e| {
                Error::Transport(format!("unexpected response from {service}: {e}"))
            })?);
            match resp.header("opc-next-page") {
                Some(next) if !next.is_empty() => page = Some(next.to_owned()),
                _ => return Ok(all),
            }
        }
    }

    /// One signed request, no retries.
    fn send(
        &self,
        method: Method,
        service: &str,
        path_and_query: &str,
        body: Option<&[u8]>,
        extra_headers: &[(&str, &str)],
    ) -> Result<Response> {
        let (host, url) = match &self.endpoint {
            Some(base) => {
                let base = base
                    .replace("{region}", &self.region)
                    .replace("{service}", service);
                let authority = base.split_once("://").map_or(base.as_str(), |(_, h)| h);
                let host = authority.split('/').next().unwrap_or(authority);
                (host.to_owned(), format!("{base}{path_and_query}"))
            }
            None => {
                let host = self.host(service);
                let url = format!("https://{host}{path_and_query}");
                (host, url)
            }
        };
        let body = if method.has_body() {
            Some(body.unwrap_or_default())
        } else {
            body
        };
        let date = httpdate::fmt_http_date(SystemTime::now());
        let signed = self.signer.sign(&Request {
            method: method.as_str(),
            host: &host,
            path_and_query,
            date: &date,
            body,
        });

        // ureq sets content-length from the body itself; ours is the same number.
        let headers: Vec<(&str, &str)> = std::iter::once(("date", date.as_str()))
            .chain(
                signed
                    .iter()
                    .filter(|(name, _)| *name != "content-length")
                    .map(|(name, value)| (*name, value.as_str())),
            )
            .chain(extra_headers.iter().copied())
            .collect();

        macro_rules! with_headers {
            ($builder:expr) => {{
                let mut builder = $builder;
                for (name, value) in &headers {
                    builder = builder.header(*name, *value);
                }
                builder
            }};
        }
        let result = match (method, body) {
            (Method::Get, _) => with_headers!(self.agent.get(&url)).call(),
            (Method::Delete, None) => with_headers!(self.agent.delete(&url)).call(),
            (Method::Delete, Some(body)) => with_headers!(self.agent.delete(&url))
                .force_send_body()
                .send(body),
            (Method::Post, body) => {
                with_headers!(self.agent.post(&url)).send(body.unwrap_or_default())
            }
            (Method::Put, body) => {
                with_headers!(self.agent.put(&url)).send(body.unwrap_or_default())
            }
        };
        let mut response = result.map_err(|e| Error::Transport(e.to_string()))?;

        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
            .collect();
        let body = response
            .body_mut()
            .read_to_vec()
            .map_err(|e| Error::Transport(e.to_string()))?;
        let response = Response {
            status,
            headers,
            body,
            service: service.to_owned(),
        };

        if (200..300).contains(&status) {
            Ok(response)
        } else {
            let text = String::from_utf8_lossy(&response.body);
            let request_id = response.request_id().map(str::to_owned);
            let mut e = api_error(status, &text, request_id);
            e.retry_after = response
                .header("retry-after")
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            Err(e.into())
        }
    }
}

fn to_json<B: Serialize + ?Sized>(body: &B) -> Result<Vec<u8>> {
    serde_json::to_vec(body).map_err(|e| Error::Transport(format!("cannot serialize body: {e}")))
}

/// Turn an error response into an [`ApiError`], whatever shape its body has.
pub(crate) fn api_error(status: u16, body: &str, request_id: Option<String>) -> ApiError {
    #[derive(serde::Deserialize)]
    struct Body {
        code: Option<String>,
        message: Option<String>,
    }
    let parsed: Option<Body> = serde_json::from_str(body).ok();
    let code = parsed.as_ref().and_then(|b| b.code.clone());
    let message = parsed.and_then(|b| b.message);
    ApiError {
        status,
        code: code.unwrap_or_else(|| format!("Http{status}")),
        message: message.unwrap_or_else(|| {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                "no message".to_owned()
            } else {
                trimmed.chars().take(200).collect()
            }
        }),
        request_id,
        retry_after: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_oci_error_bodies() {
        let e = api_error(
            500,
            r#"{"code":"InternalError","message":"Out of host capacity."}"#,
            Some("req-1".into()),
        );
        assert_eq!(e.code, "InternalError");
        assert_eq!(e.message, "Out of host capacity.");
        assert_eq!(e.request_id.as_deref(), Some("req-1"));
    }

    #[test]
    fn survives_non_json_bodies() {
        let e = api_error(502, "<html>Bad Gateway</html>", None);
        assert_eq!(e.code, "Http502");
        assert_eq!(e.message, "<html>Bad Gateway</html>");
        assert_eq!(api_error(503, "", None).message, "no message");
    }

    #[test]
    fn methods_sign_lower_case() {
        assert_eq!(Method::Get.as_str(), "get");
        assert_eq!(Method::Delete.as_str(), "delete");
        assert!(Method::Post.has_body() && Method::Put.has_body());
        assert!(!Method::Get.has_body() && !Method::Delete.has_body());
    }
}
