//! `grit http-backend` — CGI program for smart and dumb HTTP transport.
//!
//! Implements the server side of the Git HTTP protocol as a CGI program,
//! mirroring upstream `git-http-backend`. It routes a request derived from
//! `PATH_TRANSLATED` (or `GIT_PROJECT_ROOT` + `PATH_INFO`) against a table of
//! known endpoints, enforces the export / `http.getanyfile` / `http.uploadpack`
//! / `http.receivepack` access policies, serves dumb-protocol static files, and
//! proxies the smart-protocol RPC services (`upload-pack`, `receive-pack`).
//!
//!     grit http-backend
//!
//! The behaviour is exercised by `t5560-http-backend-noserver.sh` (access
//! policy and static files) and `t5562-http-backend-content-length.sh`
//! (request-body handling).

use anyhow::{anyhow, Context, Result};
use clap::Args as ClapArgs;
use flate2::read::GzDecoder;
use grit_lib::config::ConfigSet;
use grit_lib::pkt_line;
use std::env;
use std::fs;
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::grit_exe;

/// HTTP smart service endpoint kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Service {
    UploadPack,
    ReceivePack,
}

impl Service {
    /// The bare command name (`upload-pack` / `receive-pack`).
    fn command_name(self) -> &'static str {
        match self {
            Self::UploadPack => "upload-pack",
            Self::ReceivePack => "receive-pack",
        }
    }

    /// The `git-` prefixed service name used in URLs and advertisements.
    fn service_name(self) -> &'static str {
        match self {
            Self::UploadPack => "git-upload-pack",
            Self::ReceivePack => "git-receive-pack",
        }
    }

    /// Default enablement when no config is present: upload-pack is on, the rest
    /// are decided by `REMOTE_USER` (matching upstream's `signed enabled : 2`,
    /// where `1` means on and `-1` means "depends on authenticated user").
    fn default_enabled(self) -> EnabledDefault {
        match self {
            Self::UploadPack => EnabledDefault::On,
            Self::ReceivePack => EnabledDefault::ByRemoteUser,
        }
    }

    /// Expected POST `Content-Type` for this service's RPC request.
    fn request_content_type(self) -> &'static str {
        match self {
            Self::UploadPack => "application/x-git-upload-pack-request",
            Self::ReceivePack => "application/x-git-receive-pack-request",
        }
    }

    /// Content type of the RPC result body.
    fn result_content_type(self) -> &'static str {
        match self {
            Self::UploadPack => "application/x-git-upload-pack-result",
            Self::ReceivePack => "application/x-git-receive-pack-result",
        }
    }

    /// Content type of the `info/refs?service=...` advertisement body.
    fn advertisement_content_type(self) -> String {
        format!("application/x-{}-advertisement", self.service_name())
    }

    /// Parse a `git-upload-pack` / `git-receive-pack` service token.
    fn from_service_name(name: &str) -> Option<Self> {
        match name {
            "git-upload-pack" => Some(Self::UploadPack),
            "git-receive-pack" => Some(Self::ReceivePack),
            _ => None,
        }
    }
}

/// Whether a service is enabled by default before config is consulted.
#[derive(Clone, Copy, Debug)]
enum EnabledDefault {
    /// Always enabled unless config turns it off.
    On,
    /// Enabled only when `REMOTE_USER` is set (authenticated request).
    ByRemoteUser,
}

/// A fully-formed CGI response. A `None` status means the implicit `200 OK`
/// (upstream omits the `Status:` line for success); a `Some(_)` status emits an
/// explicit `Status:` line. `body` is `None` for streamed responses that have
/// already written their payload to stdout.
struct HttpResponse {
    status: Option<&'static str>,
    content_type: Option<String>,
    body: Option<Vec<u8>>,
    /// Extra raw header lines (without trailing CRLF), e.g. `no-cache` hints.
    extra_headers: Vec<String>,
}

impl HttpResponse {
    /// A non-streamed `200 OK` response with a body and content type.
    fn ok(content_type: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            status: None,
            content_type: Some(content_type.into()),
            body: Some(body),
            extra_headers: Vec::new(),
        }
    }

    /// A streamed response: headers are written, the body has already been
    /// emitted to stdout by the handler.
    fn streamed(content_type: impl Into<String>) -> Self {
        Self {
            status: None,
            content_type: Some(content_type.into()),
            body: None,
            extra_headers: Vec::new(),
        }
    }

    /// A `404 Not Found` response (no body).
    fn not_found() -> Self {
        Self {
            status: Some("404 Not Found"),
            content_type: None,
            body: Some(Vec::new()),
            extra_headers: Vec::new(),
        }
    }

    /// A `403 Forbidden` response (no body).
    fn forbidden() -> Self {
        Self {
            status: Some("403 Forbidden"),
            content_type: None,
            body: Some(Vec::new()),
            extra_headers: Vec::new(),
        }
    }

    /// A method-not-allowed / bad-request response.
    fn bad_request(allow: Option<&'static str>) -> Self {
        let http11 = env::var("SERVER_PROTOCOL").as_deref() == Ok("HTTP/1.1");
        if http11 {
            let mut extra = Vec::new();
            if let Some(allow) = allow {
                extra.push(format!("Allow: {allow}"));
            }
            Self {
                status: Some("405 Method Not Allowed"),
                content_type: None,
                body: Some(Vec::new()),
                extra_headers: extra,
            }
        } else {
            Self {
                status: Some("400 Bad Request"),
                content_type: None,
                body: Some(Vec::new()),
                extra_headers: Vec::new(),
            }
        }
    }
}

/// Arguments for `grit http-backend`.
#[derive(Debug, ClapArgs)]
#[command(about = "Server side implementation of Git over HTTP")]
pub struct Args {
    /// Stateless RPC mode (for smart HTTP).
    #[arg(long = "stateless-rpc")]
    pub stateless_rpc: bool,
}

/// Run `grit http-backend`.
///
/// Reads the CGI environment, dispatches the request, and writes the CGI
/// response. Fatal protocol/setup errors are surfaced as a `500` response with
/// the message echoed to stderr, matching the way upstream's `die_webcgi`
/// reports unexpected failures.
pub fn run(_args: Args) -> Result<()> {
    let response = match run_inner() {
        Ok(response) => response,
        Err(err) => {
            eprintln!("fatal: {err}");
            HttpResponse {
                status: Some("500 Internal Server Error"),
                content_type: Some("text/plain".to_owned()),
                body: Some(format!("fatal: {err}\n").into_bytes()),
                extra_headers: Vec::new(),
            }
        }
    };
    write_cgi_response(&response)?;
    Ok(())
}

/// Drive a single request to a response (or a fatal error).
fn run_inner() -> Result<HttpResponse> {
    let method =
        env::var("REQUEST_METHOD").map_err(|_| anyhow!("No REQUEST_METHOD from server"))?;
    let mut method = method.trim().to_ascii_uppercase();
    if method == "HEAD" {
        method = "GET".to_owned();
    }

    let dir = get_request_dir()?;
    let path = dir.path.as_str();

    // Match against the routing table; capture the repo directory prefix and
    // the per-route argument (the captured path tail).
    let Some(matched) = match_route(path) else {
        return Ok(HttpResponse::not_found());
    };

    if matched.route.method != method {
        let allow = if matched.route.method == "GET" {
            Some("GET, HEAD")
        } else {
            Some(matched.route.method)
        };
        return Ok(HttpResponse::bad_request(allow));
    }

    let repo_dir = PathBuf::from(&path[..matched.repo_end]);
    let arg = matched.arg;

    // The repository must exist and be a git directory.
    let git_dir = match resolve_git_dir(&repo_dir) {
        Some(git_dir) => git_dir,
        None => return Ok(HttpResponse::not_found()),
    };

    // Export policy: a repository is only served if `GIT_HTTP_EXPORT_ALL` is set
    // or a `git-daemon-export-ok` marker file exists in the git directory.
    if env::var_os("GIT_HTTP_EXPORT_ALL").is_none()
        && !git_dir.join("git-daemon-export-ok").exists()
    {
        return Ok(HttpResponse::not_found());
    }

    let config = load_repo_config(&git_dir);
    let policy = HttpPolicy::from_config(&config);

    (matched.route.handler)(&git_dir, &policy, &arg)
}

/// Repository-resolution result: the raw request path plus the resolved repo
/// directory boundary.
struct RequestDir {
    /// The full requested path (document-root-relative file system path).
    path: String,
}

/// Compute the request path the way upstream `getdir()` does.
///
/// When `GIT_PROJECT_ROOT` is set, `PATH_INFO` is appended to it (after an
/// alias-safety check). Otherwise `PATH_TRANSLATED` is used verbatim. One of the
/// two must be present.
fn get_request_dir() -> Result<RequestDir> {
    let root = env::var("GIT_PROJECT_ROOT").ok().filter(|s| !s.is_empty());
    let path_info = env::var("PATH_INFO").ok().filter(|s| !s.is_empty());
    let path_translated = env::var("PATH_TRANSLATED").ok().filter(|s| !s.is_empty());

    if let Some(root) = root {
        let Some(path_info) = path_info else {
            return Err(anyhow!("GIT_PROJECT_ROOT is set but PATH_INFO is not"));
        };
        if daemon_avoid_alias(&path_info) {
            return Err(anyhow!("'{path_info}': aliased"));
        }
        let mut full = root.trim_end_matches('/').to_owned();
        full.push('/');
        full.push_str(path_info.trim_start_matches('/'));
        return Ok(RequestDir { path: full });
    }

    if let Some(path) = path_translated {
        return Ok(RequestDir { path });
    }

    Err(anyhow!(
        "No GIT_PROJECT_ROOT or PATH_TRANSLATED from server"
    ))
}

/// Replicates upstream `daemon_avoid_alias`: reject paths that try to escape via
/// `..`, contain empty or `.`/`..` components, or are not absolute.
fn daemon_avoid_alias(p: &str) -> bool {
    // Must start with a single leading slash.
    let bytes = p.as_bytes();
    if bytes.is_empty() || bytes[0] != b'/' {
        return true;
    }
    // Reject a doubled leading slash ("//domain/...").
    if bytes.len() >= 2 && bytes[1] == b'/' {
        return true;
    }
    let mut sawslash_n = 0; // count of consecutive non-slash chars in segment
    let mut i = 0;
    let mut seen_component = false;
    while i < bytes.len() {
        if bytes[i] == b'/' {
            // An empty component ("//") or a "." / ".." component is rejected.
            if sawslash_n == 0 && seen_component {
                return true;
            }
            sawslash_n = 0;
            // skip duplicate slashes are caught above on next iteration
            i += 1;
            seen_component = true;
            continue;
        }
        // Detect a "." or ".." component bounded by slashes.
        if sawslash_n == 0 {
            let rest = &p[i..];
            let comp_end = rest.find('/').unwrap_or(rest.len());
            let comp = &rest[..comp_end];
            if comp == "." || comp == ".." {
                return true;
            }
        }
        sawslash_n += 1;
        i += 1;
    }
    false
}

/// A route in the service table.
struct Route {
    method: &'static str,
    matcher: fn(&str) -> Option<usize>,
    handler: fn(&Path, &HttpPolicy, &str) -> Result<HttpResponse>,
}

/// A successful route match.
struct Matched {
    route: &'static Route,
    /// Byte offset in the path where the repository directory ends (i.e. the
    /// start of the matched `/...` tail).
    repo_end: usize,
    /// The captured argument: the matched path tail, minus the leading slash.
    arg: String,
}

/// The routing table, mirroring upstream's `services[]`.
static ROUTES: &[Route] = &[
    Route {
        method: "GET",
        matcher: match_head,
        handler: get_head,
    },
    Route {
        method: "GET",
        matcher: match_info_refs,
        handler: get_info_refs,
    },
    Route {
        method: "GET",
        matcher: match_info_alternates,
        handler: get_text_file,
    },
    Route {
        method: "GET",
        matcher: match_info_http_alternates,
        handler: get_text_file,
    },
    Route {
        method: "GET",
        matcher: match_info_packs,
        handler: get_info_packs,
    },
    Route {
        method: "GET",
        matcher: match_loose_object,
        handler: get_loose_object,
    },
    Route {
        method: "GET",
        matcher: match_pack_file,
        handler: get_pack_file,
    },
    Route {
        method: "GET",
        matcher: match_idx_file,
        handler: get_idx_file,
    },
    Route {
        method: "POST",
        matcher: match_git_upload_pack,
        handler: service_rpc,
    },
    Route {
        method: "POST",
        matcher: match_git_receive_pack,
        handler: service_rpc,
    },
];

/// Find the route matching `path`. Each matcher returns the byte offset within
/// `path` of the leading `/` that begins the matched tail, so the repository
/// directory is `path[..offset]` and the argument is everything after the `/`.
fn match_route(path: &str) -> Option<Matched> {
    for route in ROUTES {
        if let Some(offset) = (route.matcher)(path) {
            let arg = path[offset + 1..].to_owned();
            return Some(Matched {
                route,
                repo_end: offset,
                arg,
            });
        }
    }
    None
}

/// Return the offset of `suffix` if `path` ends with it, else `None`. The offset
/// points at the leading `/` of `suffix`.
fn ends_with_offset(path: &str, suffix: &str) -> Option<usize> {
    if path.len() >= suffix.len() && path.ends_with(suffix) {
        Some(path.len() - suffix.len())
    } else {
        None
    }
}

fn match_head(path: &str) -> Option<usize> {
    ends_with_offset(path, "/HEAD")
}

fn match_info_refs(path: &str) -> Option<usize> {
    ends_with_offset(path, "/info/refs")
}

fn match_info_alternates(path: &str) -> Option<usize> {
    ends_with_offset(path, "/objects/info/alternates")
}

fn match_info_http_alternates(path: &str) -> Option<usize> {
    ends_with_offset(path, "/objects/info/http-alternates")
}

fn match_info_packs(path: &str) -> Option<usize> {
    ends_with_offset(path, "/objects/info/packs")
}

/// Match a loose object path: `/objects/<2 hex>/<38 or 62 hex>`.
fn match_loose_object(path: &str) -> Option<usize> {
    let idx = path.rfind("/objects/")?;
    let tail = &path[idx + "/objects/".len()..];
    let mut parts = tail.split('/');
    let dir = parts.next()?;
    let file = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if dir.len() != 2 || !is_hex(dir) {
        return None;
    }
    if (file.len() == 38 || file.len() == 62) && is_hex(file) {
        Some(idx)
    } else {
        None
    }
}

/// Match a pack file: `/objects/pack/pack-<40 or 64 hex>.pack`.
fn match_pack_file(path: &str) -> Option<usize> {
    match_pack_like(path, ".pack")
}

/// Match a pack index: `/objects/pack/pack-<40 or 64 hex>.idx`.
fn match_idx_file(path: &str) -> Option<usize> {
    match_pack_like(path, ".idx")
}

fn match_pack_like(path: &str, ext: &str) -> Option<usize> {
    let idx = path.rfind("/objects/pack/pack-")?;
    let tail = &path[idx + "/objects/pack/".len()..];
    // tail should be exactly "pack-<hex>.<ext>" with no further slashes.
    if tail.contains('/') {
        return None;
    }
    let name = tail.strip_prefix("pack-")?;
    let hex = name.strip_suffix(ext)?;
    if (hex.len() == 40 || hex.len() == 64) && is_hex(hex) {
        Some(idx)
    } else {
        None
    }
}

fn match_git_upload_pack(path: &str) -> Option<usize> {
    ends_with_offset(path, "/git-upload-pack")
}

fn match_git_receive_pack(path: &str) -> Option<usize> {
    ends_with_offset(path, "/git-receive-pack")
}

/// Whether every byte is a lowercase hex digit.
fn is_hex(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Access policy derived from repository config.
struct HttpPolicy {
    getanyfile: bool,
    upload_pack: Option<bool>,
    receive_pack: Option<bool>,
}

impl HttpPolicy {
    /// Read `http.getanyfile`, `http.uploadpack`, and `http.receivepack`.
    /// `getanyfile` defaults to `true`; the service toggles default to "unset"
    /// so their own service defaults apply.
    fn from_config(config: &ConfigSet) -> Self {
        let getanyfile = config
            .get_bool("http.getanyfile")
            .and_then(|r| r.ok())
            .unwrap_or(true);
        let upload_pack = config.get_bool("http.uploadpack").and_then(|r| r.ok());
        let receive_pack = config.get_bool("http.receivepack").and_then(|r| r.ok());
        Self {
            getanyfile,
            upload_pack,
            receive_pack,
        }
    }

    /// Whether the requested smart service is enabled, applying config overrides
    /// and the per-service defaults (`REMOTE_USER`-gated for receive-pack).
    fn service_enabled(&self, service: Service) -> bool {
        let configured = match service {
            Service::UploadPack => self.upload_pack,
            Service::ReceivePack => self.receive_pack,
        };
        if let Some(enabled) = configured {
            return enabled;
        }
        match service.default_enabled() {
            EnabledDefault::On => true,
            EnabledDefault::ByRemoteUser => env::var("REMOTE_USER")
                .map(|u| !u.is_empty())
                .unwrap_or(false),
        }
    }
}

/// Resolve the git directory for a requested repository path, or `None` if it is
/// not a git repository. Accepts both a bare repo directory and a working tree
/// containing a `.git` directory.
fn resolve_git_dir(repo_dir: &Path) -> Option<PathBuf> {
    if is_git_dir(repo_dir) {
        return Some(repo_dir.to_path_buf());
    }
    let dot_git = repo_dir.join(".git");
    if is_git_dir(&dot_git) {
        return Some(dot_git);
    }
    None
}

/// Heuristic for a git directory: it has `HEAD` and an `objects` directory,
/// matching upstream's `enter_repo`/`is_git_directory` essentials.
fn is_git_dir(dir: &Path) -> bool {
    dir.join("HEAD").exists() && dir.join("objects").is_dir()
}

/// Load repository config (local file only) for policy decisions. Errors are
/// treated as "no config", so defaults apply.
fn load_repo_config(git_dir: &Path) -> ConfigSet {
    ConfigSet::load_repo_local_only(git_dir).unwrap_or_default()
}

/// Enforce `http.getanyfile`: returns `Err(forbidden)` when disabled.
fn select_getanyfile(policy: &HttpPolicy) -> std::result::Result<(), HttpResponse> {
    if policy.getanyfile {
        Ok(())
    } else {
        Err(HttpResponse::forbidden())
    }
}

/// Serve a file from within the git directory as a static download. Returns a
/// `404` response when the file is absent.
fn send_local_file(git_dir: &Path, content_type: &str, name: &str) -> Result<HttpResponse> {
    let p = git_dir.join(name);
    match fs::read(&p) {
        Ok(bytes) => Ok(HttpResponse::ok(content_type, bytes)),
        Err(_) => Ok(HttpResponse::not_found()),
    }
}

/// `GET /<repo>/objects/info/alternates` and `.../http-alternates` — text file.
fn get_text_file(git_dir: &Path, policy: &HttpPolicy, arg: &str) -> Result<HttpResponse> {
    if let Err(resp) = select_getanyfile(policy) {
        return Ok(resp);
    }
    send_local_file(git_dir, "text/plain", arg)
}

/// `GET /<repo>/objects/<2>/<rest>` — a loose object.
fn get_loose_object(git_dir: &Path, policy: &HttpPolicy, arg: &str) -> Result<HttpResponse> {
    if let Err(resp) = select_getanyfile(policy) {
        return Ok(resp);
    }
    send_local_file(git_dir, "application/x-git-loose-object", arg)
}

/// `GET /<repo>/objects/pack/pack-<hex>.pack` — a packfile.
fn get_pack_file(git_dir: &Path, policy: &HttpPolicy, arg: &str) -> Result<HttpResponse> {
    if let Err(resp) = select_getanyfile(policy) {
        return Ok(resp);
    }
    send_local_file(git_dir, "application/x-git-packed-objects", arg)
}

/// `GET /<repo>/objects/pack/pack-<hex>.idx` — a pack index.
fn get_idx_file(git_dir: &Path, policy: &HttpPolicy, arg: &str) -> Result<HttpResponse> {
    if let Err(resp) = select_getanyfile(policy) {
        return Ok(resp);
    }
    send_local_file(git_dir, "application/x-git-packed-objects-toc", arg)
}

/// `GET /<repo>/HEAD` — dumb HEAD pointer (resolved ref or OID).
fn get_head(git_dir: &Path, policy: &HttpPolicy, _arg: &str) -> Result<HttpResponse> {
    if let Err(resp) = select_getanyfile(policy) {
        return Ok(resp);
    }
    // The dumb HEAD text is just the raw HEAD file's symref/OID line; serving the
    // raw file is sufficient for clients and matches the file existing in any
    // valid repository.
    send_local_file(git_dir, "text/plain", "HEAD")
}

/// `GET /<repo>/objects/info/packs` — list local packs in dumb format.
fn get_info_packs(git_dir: &Path, policy: &HttpPolicy, _arg: &str) -> Result<HttpResponse> {
    if let Err(resp) = select_getanyfile(policy) {
        return Ok(resp);
    }
    let pack_dir = git_dir.join("objects").join("pack");
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(&pack_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("pack-") && name.ends_with(".pack") {
                    names.push(name.to_owned());
                }
            }
        }
    }
    names.sort();
    let mut body = String::new();
    for name in names {
        body.push_str("P ");
        body.push_str(&name);
        body.push('\n');
    }
    body.push('\n');
    Ok(HttpResponse::ok(
        "text/plain; charset=utf-8",
        body.into_bytes(),
    ))
}

/// `GET /<repo>/info/refs[?service=...]` — either the smart advertisement (when
/// `service=` is present) or the dumb refs listing.
fn get_info_refs(git_dir: &Path, policy: &HttpPolicy, _arg: &str) -> Result<HttpResponse> {
    let query = env::var("QUERY_STRING").unwrap_or_default();
    let service_name = query_param(&query, "service");

    if let Some(service_name) = service_name {
        let Some(service) = Service::from_service_name(&service_name) else {
            return Ok(HttpResponse::forbidden());
        };
        if !policy.service_enabled(service) {
            return Ok(HttpResponse::forbidden());
        }
        return run_info_refs_advertisement(git_dir, service);
    }

    // Dumb protocol: gated by getanyfile, served from info/refs if present, else
    // a generated listing.
    if let Err(resp) = select_getanyfile(policy) {
        return Ok(resp);
    }
    match fs::read(git_dir.join("info").join("refs")) {
        Ok(bytes) => Ok(HttpResponse::ok("text/plain", bytes)),
        Err(_) => Ok(HttpResponse::ok(
            "text/plain",
            generate_dumb_info_refs(git_dir),
        )),
    }
}

/// Build a dumb-protocol `info/refs` body (`<oid>\t<refname>` per line) by
/// reading loose refs and `packed-refs`.
fn generate_dumb_info_refs(git_dir: &Path) -> Vec<u8> {
    let mut refs: Vec<(String, String)> = Vec::new();
    collect_loose_refs(&git_dir.join("refs"), git_dir, &mut refs);
    if let Ok(packed) = fs::read_to_string(git_dir.join("packed-refs")) {
        for line in packed.lines() {
            if line.starts_with('#') || line.starts_with('^') {
                continue;
            }
            if let Some((oid, name)) = line.split_once(' ') {
                if name.starts_with("refs/") {
                    refs.push((name.to_owned(), oid.to_owned()));
                }
            }
        }
    }
    refs.sort();
    refs.dedup_by(|a, b| a.0 == b.0);
    let mut body = String::new();
    for (name, oid) in refs {
        body.push_str(&oid);
        body.push('\t');
        body.push_str(&name);
        body.push('\n');
    }
    body.into_bytes()
}

/// Recursively collect loose refs under `dir`, recording `(refname, oid)`.
fn collect_loose_refs(dir: &Path, git_dir: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_loose_refs(&path, git_dir, out);
        } else if let Ok(rel) = path.strip_prefix(git_dir) {
            if let Some(name) = rel.to_str() {
                if let Ok(content) = fs::read_to_string(&path) {
                    let oid = content.trim();
                    if oid.len() >= 40 && !oid.starts_with("ref:") {
                        out.push((name.to_owned(), oid.to_owned()));
                    }
                }
            }
        }
    }
}

/// Run the smart `info/refs` advertisement for `service`, streaming the
/// `# service=...` banner plus the ref advertisement to stdout.
fn run_info_refs_advertisement(git_dir: &Path, service: Service) -> Result<HttpResponse> {
    let output = Command::new(grit_exe::grit_executable())
        .arg(service.command_name())
        .arg("--http-backend-info-refs")
        .arg(git_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    // Write headers (implicit 200), then the banner and the advertisement. Even
    // if the helper fails, the status stays 200 — upstream commits to 200 before
    // running the service.
    let banner = format!("# service={}\n", service.service_name());
    let mut body: Vec<u8> = Vec::new();
    write_pkt_line(&mut body, banner.as_bytes());
    body.extend_from_slice(b"0000");
    if let Ok(out) = output {
        body.extend_from_slice(&out.stdout);
    }

    let mut stdout = io::stdout().lock();
    write_headers(
        &mut stdout,
        None,
        Some(&service.advertisement_content_type()),
        &[],
    )?;
    stdout.write_all(&body)?;
    stdout.flush()?;
    Ok(HttpResponse::streamed(service.advertisement_content_type()))
}

/// `POST /<repo>/git-upload-pack` or `.../git-receive-pack` — proxy the RPC.
///
/// Enforces the service access policy and content type, validates that the
/// request body is a well-formed (possibly empty-of-commands but flush-bearing)
/// RPC request, then runs the service and returns its result body. A malformed
/// request (empty or truncated) surfaces as a fatal `500` so callers see the
/// `fatal:` diagnostic on stderr.
fn service_rpc(git_dir: &Path, policy: &HttpPolicy, arg: &str) -> Result<HttpResponse> {
    let Some(service) = Service::from_service_name(arg) else {
        return Ok(HttpResponse::forbidden());
    };
    if !policy.service_enabled(service) {
        return Ok(HttpResponse::forbidden());
    }

    // Validate the request content type; a mismatch is a fatal protocol error.
    let content_type = env::var("CONTENT_TYPE").unwrap_or_default();
    if !content_type
        .to_ascii_lowercase()
        .starts_with(service.request_content_type())
    {
        return Err(anyhow!(
            "CONTENT_TYPE for {} is {content_type:?}, expected {:?}",
            service.command_name(),
            service.request_content_type()
        ));
    }

    let body = read_request_body()?;
    validate_rpc_request(service, &body)?;

    let output = run_service_command(service, git_dir, &body)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if stderr.is_empty() {
            return Err(anyhow!(
                "{} failed with status {}",
                service.command_name(),
                output.status
            ));
        }
        return Err(anyhow!("{stderr}"));
    }

    Ok(HttpResponse::ok(
        service.result_content_type(),
        output.stdout,
    ))
}

/// Run the RPC service subprocess, feeding `body` to its stdin and capturing its
/// output. The service is invoked the same way upstream's stateless RPC path
/// runs it (advertise refs, then read the request from stdin).
fn run_service_command(service: Service, git_dir: &Path, body: &[u8]) -> Result<Output> {
    let mut child = Command::new(grit_exe::grit_executable())
        .arg(service.command_name())
        .arg(git_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {}", service.command_name()))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(body)
            .with_context(|| format!("failed to write body to {}", service.command_name()))?;
    }
    child
        .wait_with_output()
        .with_context(|| format!("failed to wait for {}", service.command_name()))
}

/// Validate an RPC request body, returning an error for an empty or truncated
/// request. A flush-only request (`0000`, no commands/wants) is accepted: the
/// service simply has nothing to do and exits cleanly with a `200`.
fn validate_rpc_request(service: Service, body: &[u8]) -> Result<()> {
    match service {
        Service::UploadPack => validate_upload_pack_request(body),
        Service::ReceivePack => validate_receive_pack_request(body),
    }
}

/// Validate an `upload-pack` request: the pkt-line stream must parse cleanly and
/// terminate in a flush packet. An empty body (no flush) or a truncated pkt-line
/// is rejected.
fn validate_upload_pack_request(body: &[u8]) -> Result<()> {
    let mut cursor = Cursor::new(body);
    let mut saw_flush = false;

    loop {
        match pkt_line::read_packet(&mut cursor).context("invalid upload-pack request")? {
            None => break,
            Some(pkt_line::Packet::Flush) => {
                saw_flush = true;
                break;
            }
            Some(_) => {}
        }
    }

    if !saw_flush {
        return Err(anyhow!("upload-pack request is missing flush packet"));
    }
    Ok(())
}

/// Validate a `receive-pack` request: the pkt-line command stream must parse and
/// terminate in a flush. If any command line was present, a `PACK` payload must
/// follow. A flush-only request (no commands) is accepted.
fn validate_receive_pack_request(body: &[u8]) -> Result<()> {
    let mut cursor = Cursor::new(body);
    let mut saw_update_line = false;
    let mut saw_flush = false;

    loop {
        match pkt_line::read_packet(&mut cursor).context("invalid receive-pack request")? {
            None => break,
            Some(pkt_line::Packet::Data(_)) => saw_update_line = true,
            Some(pkt_line::Packet::Flush) => {
                saw_flush = true;
                break;
            }
            Some(_) => {}
        }
    }

    if !saw_flush {
        return Err(anyhow!("receive-pack request is missing flush packet"));
    }

    // A flush-only request has no commands and therefore no pack to send.
    if !saw_update_line {
        return Ok(());
    }

    let offset = cursor.position() as usize;
    let pack = &body[offset..];
    if pack.len() <= 12 || !pack.starts_with(b"PACK") {
        return Err(anyhow!("receive-pack request is missing PACK payload"));
    }
    Ok(())
}

/// Read the POST request body, honoring `CONTENT_LENGTH` and optional gzip
/// `Content-Encoding`.
fn read_request_body() -> Result<Vec<u8>> {
    let content_length = parse_content_length()?;
    let mut stdin = io::stdin().lock();
    let raw = match content_length {
        Some(expected) => {
            let mut body = vec![0_u8; expected];
            stdin
                .read_exact(&mut body)
                .with_context(|| format!("failed to read CONTENT_LENGTH bytes ({expected})"))?;
            body
        }
        None => {
            let mut body = Vec::new();
            stdin
                .read_to_end(&mut body)
                .context("failed to read request body")?;
            body
        }
    };
    decode_request_body(raw)
}

/// Parse and validate `CONTENT_LENGTH`.
fn parse_content_length() -> Result<Option<usize>> {
    let Some(raw) = env::var("CONTENT_LENGTH").ok() else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let parsed = trimmed
        .parse::<u128>()
        .with_context(|| format!("invalid CONTENT_LENGTH: {trimmed}"))?;
    if parsed > isize::MAX as u128 || parsed > usize::MAX as u128 {
        return Err(anyhow!(
            "invalid CONTENT_LENGTH: {trimmed} does not fit in ssize_t"
        ));
    }
    Ok(Some(parsed as usize))
}

/// Decode the raw request body according to `Content-Encoding` (`identity` or
/// `gzip`).
fn decode_request_body(encoded: Vec<u8>) -> Result<Vec<u8>> {
    let encoding = env::var("HTTP_CONTENT_ENCODING")
        .or_else(|_| env::var("CONTENT_ENCODING"))
        .unwrap_or_else(|_| "identity".to_owned())
        .trim()
        .to_ascii_lowercase();

    match encoding.as_str() {
        "" | "identity" => Ok(encoded),
        "gzip" | "x-gzip" => {
            let mut decoder = GzDecoder::new(encoded.as_slice());
            let mut decoded = Vec::new();
            decoder
                .read_to_end(&mut decoded)
                .context("failed to decode gzip request body")?;
            Ok(decoded)
        }
        other => Err(anyhow!("unsupported Content-Encoding: {other}")),
    }
}

/// Extract a query parameter value by name (first occurrence).
fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == name {
            Some(url_decode(v))
        } else {
            None
        }
    })
}

/// Minimal URL decoding (`%XX` and `+`).
fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8 as char);
                    i += 3;
                } else {
                    out.push('%');
                    i += 1;
                }
            }
            b => {
                out.push(b as char);
                i += 1;
            }
        }
    }
    out
}

/// Write a single pkt-line (4-byte hex length prefix + payload) into `buf`.
fn write_pkt_line(buf: &mut Vec<u8>, payload: &[u8]) {
    let len = payload.len() + 4;
    buf.extend_from_slice(format!("{len:04x}").as_bytes());
    buf.extend_from_slice(payload);
}

/// Write the CGI headers: optional `Status`, optional `Content-Type`, any extra
/// header lines, then the blank separator line.
fn write_headers(
    out: &mut impl Write,
    status: Option<&str>,
    content_type: Option<&str>,
    extra: &[String],
) -> Result<()> {
    if let Some(status) = status {
        write!(out, "Status: {status}\r\n")?;
    }
    if let Some(ct) = content_type {
        write!(out, "Content-Type: {ct}\r\n")?;
    }
    for line in extra {
        write!(out, "{line}\r\n")?;
    }
    write!(out, "\r\n")?;
    Ok(())
}

/// Write a fully-buffered CGI response (status, headers, body). Streamed
/// responses (those with `body == None`) have already written their payload, so
/// only the no-op return remains.
fn write_cgi_response(response: &HttpResponse) -> Result<()> {
    let Some(body) = &response.body else {
        return Ok(());
    };
    let mut out = io::stdout().lock();
    if let Some(status) = response.status {
        write!(out, "Status: {status}\r\n")?;
    }
    if let Some(ct) = &response.content_type {
        write!(out, "Content-Type: {ct}\r\n")?;
    }
    write!(out, "Content-Length: {}\r\n", body.len())?;
    for line in &response.extra_headers {
        write!(out, "{line}\r\n")?;
    }
    write!(out, "\r\n")?;
    out.write_all(body)?;
    out.flush()?;
    Ok(())
}
