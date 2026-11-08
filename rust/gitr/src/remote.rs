//! Git LFS basic transfers and file locking over HTTPS.
//!
//! Repository credentials apply only to the configured endpoint's origin. A
//! batch action may carry separate headers for its storage service. Redirects
//! are never followed, and errors do not expose URLs, tokens, or server text.
use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use ureq::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, Response, header};
use ureq::{Agent, AsSendBody, Body};
use url::{Host, Url};

use crate::lfs::{Pointer, Store};

const LFS_JSON: &str = "application/vnd.git-lfs+json";
const BINARY: &str = "application/octet-stream";
const MAX_JSON: u64 = 1024 * 1024;
const MAX_LOCK_PAGES: usize = 1000;
const MAX_LOCK_BYTES: usize = 32 * 1024 * 1024;
const MAX_CURSOR_BYTES: usize = 4096;

/// A client for one explicit LFS server endpoint, such as a URL ending in
/// `/owner/repo.git/info/lfs`. It does not discover endpoints or credentials.
pub struct Client {
    endpoint: Url,
    authorization: Option<HeaderValue>,
    agent: Agent,
    allow_insecure_loopback: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LockOwner {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Lock {
    pub id: String,
    pub path: String,
    pub locked_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<LockOwner>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct VerifiedLocks {
    pub ours: Vec<Lock>,
    pub theirs: Vec<Lock>,
}

#[derive(Deserialize)]
struct BatchResponse {
    transfer: Option<String>,
    hash_algo: Option<String>,
    objects: Vec<BatchObject>,
}

#[derive(Deserialize)]
struct BatchObject {
    oid: String,
    size: u64,
    error: Option<ObjectError>,
    actions: Option<BTreeMap<String, Action>>,
}

#[derive(Deserialize)]
struct ObjectError {
    code: u16,
}

#[derive(Deserialize)]
struct Action {
    href: String,
    #[serde(default)]
    header: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct LockResponse {
    lock: Lock,
}

#[derive(Deserialize)]
struct LockPage {
    locks: Vec<Lock>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct VerifyPage {
    ours: Vec<Lock>,
    theirs: Vec<Lock>,
    next_cursor: Option<String>,
}

impl Client {
    /// HTTPS is required unless an explicit test/development opt-in permits
    /// HTTP to a literal loopback IP address. URL credentials are not accepted.
    pub fn new(
        endpoint: &str,
        authorization: Option<String>,
        allow_insecure_loopback: bool,
    ) -> Result<Self> {
        let endpoint = parse_url(endpoint, allow_insecure_loopback)?;
        ensure!(
            endpoint.query().is_none(),
            "LFS endpoint must not have a query"
        );
        let authorization = authorization
            .map(|value| {
                let mut header = HeaderValue::from_str(&value)
                    .map_err(|_| anyhow!("invalid LFS authorization header"))?;
                header.set_sensitive(true);
                Ok::<HeaderValue, anyhow::Error>(header)
            })
            .transpose()?;
        let agent = Agent::config_builder()
            .https_only(!allow_insecure_loopback)
            .max_redirects(0)
            .http_status_as_error(false)
            // Do not route the explicit loopback exception through an ambient
            // proxy. This initial client uses direct HTTPS connections.
            .proxy(None)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_per_read(Some(Duration::from_secs(120)))
            .timeout_per_write(Some(Duration::from_secs(120)))
            .build()
            .into();
        Ok(Self {
            endpoint,
            authorization,
            agent,
            allow_insecure_loopback,
        })
    }

    /// Fetch and verify an object before the store publishes it. The response
    /// is streamed with a hard upper bound of the expected size plus one byte.
    pub fn download(&self, store: &Store, pointer: &Pointer) -> Result<()> {
        validate_pointer(pointer)?;
        let limit = pointer
            .size
            .checked_add(1)
            .context("LFS object size is too large")?;
        let object = self.batch("download", pointer)?;
        let action = object
            .actions
            .as_ref()
            .and_then(|actions| actions.get("download"))
            .context("LFS batch response has no download action")?;
        let url = self.action_url(action)?;
        let headers = self.headers(&url, BINARY, Some(action), None)?;
        let response = self.request(Method::GET, &url, headers, (), "download")?;
        require_status(&response, 200, "download")?;
        if let Some(length) = response.headers().get(header::CONTENT_LENGTH) {
            let length = length
                .to_str()
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .context("LFS download has an invalid content length")?;
            ensure!(
                length == pointer.size,
                "LFS download content length does not match the pointer"
            );
        }
        let reader = NetworkReader(response.into_body().into_reader()).take(limit);
        store.ingest(reader, Some(pointer))?;
        Ok(())
    }

    /// Upload a verified cache file as a streaming PUT, then perform the
    /// server's optional verification action. No object-sized buffer is used.
    pub fn upload(&self, store: &Store, pointer: &Pointer) -> Result<()> {
        validate_pointer(pointer)?;
        let file = store.open_verified(pointer)?;
        let object = self.batch("upload", pointer)?;
        let Some(actions) = object.actions.filter(|actions| !actions.is_empty()) else {
            // The batch protocol omits actions when the server has the object.
            return Ok(());
        };
        let action = actions
            .get("upload")
            .context("LFS batch response has no upload action")?;
        let url = self.action_url(action)?;
        let headers = self.headers(&url, BINARY, Some(action), Some(pointer.size))?;
        let response = self.request(Method::PUT, &url, headers, file, "upload")?;
        ensure!(
            response.status().is_success(),
            "LFS upload returned HTTP {}",
            response.status().as_u16()
        );
        if let Some(action) = actions.get("verify") {
            let url = self.action_url(action)?;
            let response = self.post_json(
                &url,
                &json!({ "oid": pointer.oid, "size": pointer.size }),
                Some(action),
                "upload verification",
            )?;
            require_status(&response, 200, "upload verification")?;
        }
        Ok(())
    }

    /// Acquire a server-enforced exclusive lock for a repository-relative path.
    pub fn lock(&self, path: &str) -> Result<Lock> {
        validate_lock_path(path)?;
        let response = self.post_json(
            &self.api_url(&["locks"])?,
            &json!({ "path": path }),
            None,
            "lock creation",
        )?;
        require_status(&response, 201, "lock creation")?;
        let result: LockResponse = read_json(response, "lock creation")?;
        validate_lock(&result.lock)?;
        ensure!(
            result.lock.path == path,
            "LFS lock response names a different path"
        );
        Ok(result.lock)
    }

    /// List every page, with bounded accumulation and cursor-cycle detection.
    pub fn list_locks(&self) -> Result<Vec<Lock>> {
        let mut locks = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        let mut total_bytes = 0;
        for _ in 0..MAX_LOCK_PAGES {
            let mut url = self.api_url(&["locks"])?;
            url.query_pairs_mut().append_pair("limit", "100");
            if let Some(value) = &cursor {
                url.query_pairs_mut().append_pair("cursor", value);
            }
            let headers = self.headers(&url, LFS_JSON, None, None)?;
            let response = self.request(Method::GET, &url, headers, (), "lock listing")?;
            require_status(&response, 200, "lock listing")?;
            let page: LockPage = read_json(response, "lock listing")?;
            account_locks(&page.locks, &mut total_bytes)?;
            locks.extend(page.locks);
            cursor = next_cursor(page.next_cursor, &mut seen)?;
            if cursor.is_none() {
                return Ok(locks);
            }
        }
        bail!("LFS lock listing exceeded the page limit")
    }

    /// Return all owned and foreign locks. A caller must compare `theirs` with
    /// paths it plans to publish; fetching this list alone does not guard pushes.
    pub fn verify_locks(&self) -> Result<VerifiedLocks> {
        let mut result = VerifiedLocks::default();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        let mut total_bytes = 0;
        for _ in 0..MAX_LOCK_PAGES {
            let mut body = json!({ "limit": 100 });
            if let Some(value) = &cursor {
                body["cursor"] = json!(value);
            }
            let response = self.post_json(
                &self.api_url(&["locks", "verify"])?,
                &body,
                None,
                "lock verification",
            )?;
            require_status(&response, 200, "lock verification")?;
            let page: VerifyPage = read_json(response, "lock verification")?;
            account_locks(&page.ours, &mut total_bytes)?;
            account_locks(&page.theirs, &mut total_bytes)?;
            result.ours.extend(page.ours);
            result.theirs.extend(page.theirs);
            cursor = next_cursor(page.next_cursor, &mut seen)?;
            if cursor.is_none() {
                return Ok(result);
            }
        }
        bail!("LFS lock verification exceeded the page limit")
    }

    /// Remove a lock. Breaking another user's lock requires explicit `force`.
    pub fn unlock(&self, id: &str, force: bool) -> Result<Lock> {
        validate_lock_id(id)?;
        let response = self.post_json(
            &self.api_url(&["locks", id, "unlock"])?,
            &json!({ "force": force }),
            None,
            "unlock",
        )?;
        require_status(&response, 200, "unlock")?;
        let result: LockResponse = read_json(response, "unlock")?;
        validate_lock(&result.lock)?;
        ensure!(
            result.lock.id == id,
            "LFS unlock response names a different lock"
        );
        Ok(result.lock)
    }

    fn batch(&self, operation: &str, pointer: &Pointer) -> Result<BatchObject> {
        let response = self.post_json(
            &self.api_url(&["objects", "batch"])?,
            &json!({
                "operation": operation,
                "transfers": ["basic"],
                "hash_algo": "sha256",
                "objects": [{ "oid": pointer.oid, "size": pointer.size }]
            }),
            None,
            "batch",
        )?;
        require_status(&response, 200, "batch")?;
        let mut batch: BatchResponse = read_json(response, "batch")?;
        ensure!(
            batch.transfer.as_deref().unwrap_or("basic") == "basic",
            "LFS server selected an unsupported transfer adapter"
        );
        ensure!(
            batch.hash_algo.as_deref().unwrap_or("sha256") == "sha256",
            "LFS server selected an unsupported hash algorithm"
        );
        ensure!(
            batch.objects.len() == 1,
            "LFS batch response must describe exactly the requested object"
        );
        let object = batch.objects.pop().context("LFS batch response is empty")?;
        ensure!(
            object.oid == pointer.oid && object.size == pointer.size,
            "LFS batch response does not match the requested object"
        );
        if let Some(error) = &object.error {
            bail!("LFS server rejected the object with code {}", error.code);
        }
        Ok(object)
    }

    fn api_url(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.endpoint.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow!("invalid LFS endpoint path"))?
            .pop_if_empty()
            .extend(segments.iter().copied());
        Ok(url)
    }

    fn action_url(&self, action: &Action) -> Result<Url> {
        parse_url(&action.href, self.allow_insecure_loopback)
    }

    fn headers(
        &self,
        url: &Url,
        media_type: &'static str,
        action: Option<&Action>,
        length: Option<u64>,
    ) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static(media_type));
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(media_type));
        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        if let Some(length) = length {
            headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
        }
        if url.origin() == self.endpoint.origin()
            && let Some(value) = &self.authorization
        {
            headers.insert(header::AUTHORIZATION, value.clone());
        }
        if let Some(action) = action {
            let mut seen = HashSet::new();
            for (name, value) in &action.header {
                let name = HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| anyhow!("LFS action has an invalid header name"))?;
                ensure!(
                    seen.insert(name.clone()),
                    "LFS action repeats a header name"
                );
                // The action may set signed application headers, including its
                // own Authorization, but cannot redirect or reframe a request.
                ensure!(
                    !matches!(
                        name.as_str(),
                        "host"
                            | "connection"
                            | "proxy-authorization"
                            | "proxy-connection"
                            | "transfer-encoding"
                            | "upgrade"
                            | "trailer"
                            | "te"
                    ),
                    "LFS action contains a prohibited transport header"
                );
                if name == header::CONTENT_LENGTH {
                    ensure!(
                        length.is_some() && value.parse::<u64>().ok() == length,
                        "LFS action content length conflicts with the request"
                    );
                }
                let mut value = HeaderValue::from_str(value)
                    .map_err(|_| anyhow!("LFS action has an invalid header value"))?;
                value.set_sensitive(true);
                headers.insert(name, value);
            }
        }
        Ok(headers)
    }

    fn request(
        &self,
        method: Method,
        url: &Url,
        headers: HeaderMap,
        body: impl AsSendBody,
        purpose: &str,
    ) -> Result<Response<Body>> {
        let mut request = Request::builder()
            .method(method)
            .uri(url.as_str())
            .body(body)
            .map_err(|_| anyhow!("invalid LFS {purpose} request"))?;
        *request.headers_mut() = headers;
        // Underlying errors can contain a signed URL. Do not retain that error
        // as an anyhow cause or interpolate untrusted server response text.
        self.agent
            .run(request)
            .map_err(|_| anyhow!("LFS {purpose} transport failed"))
    }

    fn post_json(
        &self,
        url: &Url,
        body: &impl Serialize,
        action: Option<&Action>,
        purpose: &str,
    ) -> Result<Response<Body>> {
        let body =
            serde_json::to_vec(body).map_err(|_| anyhow!("cannot encode LFS {purpose} request"))?;
        ensure!(
            body.len() as u64 <= MAX_JSON,
            "LFS {purpose} request exceeds the JSON limit"
        );
        let headers = self.headers(url, LFS_JSON, action, Some(body.len() as u64))?;
        self.request(Method::POST, url, headers, body, purpose)
    }
}

fn validate_pointer(pointer: &Pointer) -> Result<()> {
    Pointer::new(pointer.oid.clone(), pointer.size)?;
    Ok(())
}

fn parse_url(value: &str, allow_insecure_loopback: bool) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| anyhow!("invalid LFS URL"))?;
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "LFS URL credentials are prohibited; use an authorization header"
    );
    ensure!(url.fragment().is_none(), "LFS URL fragments are prohibited");
    ensure!(url.host().is_some(), "LFS URL must have a host");
    let loopback = match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && allow_insecure_loopback && loopback),
        "LFS requires HTTPS; explicit development mode permits HTTP only to a literal loopback IP"
    );
    Ok(url)
}

fn require_status(response: &Response<Body>, expected: u16, purpose: &str) -> Result<()> {
    ensure!(
        response.status().as_u16() == expected,
        "LFS {purpose} returned HTTP {}",
        response.status().as_u16()
    );
    Ok(())
}

fn read_json<T: DeserializeOwned>(response: Response<Body>, purpose: &str) -> Result<T> {
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_JSON + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow!("LFS {purpose} response could not be read"))?;
    ensure!(
        bytes.len() as u64 <= MAX_JSON,
        "LFS {purpose} response exceeds the JSON limit"
    );
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("LFS {purpose} returned invalid protocol JSON"))
}

fn validate_lock_path(path: &str) -> Result<()> {
    let drive_prefix = path.as_bytes().get(1) == Some(&b':');
    ensure!(
        !path.is_empty()
            && !path.starts_with('/')
            && !path.contains(['\\', '\0'])
            && !drive_prefix
            && path.split('/').all(|part| !matches!(part, "" | "." | "..")),
        "LFS lock path must be a normalized repository-relative path"
    );
    Ok(())
}

fn validate_lock_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty() && !matches!(id, "." | "..") && !id.chars().any(char::is_control),
        "invalid LFS lock ID"
    );
    Ok(())
}

fn validate_lock(lock: &Lock) -> Result<()> {
    validate_lock_id(&lock.id)?;
    validate_lock_path(&lock.path)?;
    ensure!(
        !lock.locked_at.is_empty(),
        "LFS lock response has no timestamp"
    );
    Ok(())
}

fn account_locks(locks: &[Lock], bytes: &mut usize) -> Result<()> {
    for lock in locks {
        validate_lock(lock)?;
        let size = std::mem::size_of::<Lock>()
            .saturating_add(lock.id.len())
            .saturating_add(lock.path.len())
            .saturating_add(lock.locked_at.len())
            .saturating_add(lock.owner.as_ref().map_or(0, |owner| owner.name.len()));
        *bytes = bytes.saturating_add(size);
        ensure!(
            *bytes <= MAX_LOCK_BYTES,
            "LFS locks exceed the result memory limit"
        );
    }
    Ok(())
}

fn next_cursor(cursor: Option<String>, seen: &mut HashSet<String>) -> Result<Option<String>> {
    let cursor = cursor.filter(|value| !value.is_empty());
    if let Some(value) = &cursor {
        ensure!(
            value.len() <= MAX_CURSOR_BYTES,
            "LFS lock cursor exceeds the size limit"
        );
        ensure!(
            seen.insert(value.clone()),
            "LFS lock server repeated a pagination cursor"
        );
    }
    Ok(cursor)
}

/// Keep read errors safe when the store adds its own I/O context.
struct NetworkReader<R>(R);

impl<R: Read> Read for NetworkReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0
            .read(bytes)
            .map_err(|error| io::Error::new(error.kind(), "LFS object stream was interrupted"))
    }
}
