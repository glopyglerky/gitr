//! Real HTTP exchanges against isolated local servers. These tests never
//! contact a Git hosting service or use credentials from the environment.
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gitr::lfs::{Pointer, Store};
use gitr::remote::Client;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

#[derive(Clone, Debug)]
struct Observed {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    length: Option<usize>,
}

impl Reply {
    fn bytes(body: impl Into<Vec<u8>>) -> Self {
        let body = body.into();
        Self {
            status: 200,
            headers: vec![("Content-Type".into(), "application/octet-stream".into())],
            length: Some(body.len()),
            body,
        }
    }

    fn json(status: u16, value: Value) -> Self {
        let mut reply = Self::bytes(serde_json::to_vec(&value).unwrap());
        reply.status = status;
        reply.headers = vec![("Content-Type".into(), "application/vnd.git-lfs+json".into())];
        reply
    }

    fn redirect(destination: &str) -> Self {
        let mut reply = Self::bytes(Vec::new());
        reply.status = 307;
        reply.headers.push(("Location".into(), destination.into()));
        reply
    }

    fn without_length(mut self) -> Self {
        self.length = None;
        self
    }

    fn write(self, stream: &mut TcpStream) -> io::Result<()> {
        write!(
            stream,
            "HTTP/1.1 {} Fixture\r\nConnection: close\r\n",
            self.status
        )?;
        if let Some(length) = self.length {
            write!(stream, "Content-Length: {length}\r\n")?;
        }
        for (name, value) in self.headers {
            write!(stream, "{name}: {value}\r\n")?;
        }
        stream.write_all(b"\r\n")?;
        stream.write_all(&self.body)
    }
}

struct Server {
    base: String,
    observed: Arc<Mutex<Vec<Observed>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    fn new(make_replies: impl FnOnce(&str) -> Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mut replies: VecDeque<_> = make_replies(&base).into();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_observed = observed.clone();
        let thread_stop = stop.clone();
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        if let Ok(request) = read_request(&mut stream) {
                            thread_observed.lock().unwrap().push(request);
                            let response = replies.pop_front().unwrap_or_else(|| {
                                Reply::json(500, json!({ "message": "unexpected request" }))
                            });
                            // An oversized-body test intentionally closes early.
                            let _ = response.write(&mut stream);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            base,
            observed,
            stop,
            thread: Some(thread),
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/repo.git/info/lfs", self.base)
    }

    fn requests(&self) -> Vec<Observed> {
        self.observed.lock().unwrap().clone()
    }

    fn client(&self) -> Client {
        Client::new(
            &self.endpoint(),
            Some("Bearer repository-secret".into()),
            true,
        )
        .unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> io::Result<Observed> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut fields = line.split_whitespace();
    let method = fields.next().unwrap_or_default().to_owned();
    let target = fields.next().unwrap_or_default().to_owned();
    let mut headers = BTreeMap::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.split_once(':').ok_or(io::ErrorKind::InvalidData)?;
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    // A file upload must carry its exact Content-Length instead of buffering
    // the whole object or sending an unbounded chunked body.
    if headers.contains_key("transfer-encoding") {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let length = headers.get("content-length").map_or(Ok(0), |value| {
        value
            .parse::<usize>()
            .map_err(|_| io::ErrorKind::InvalidData)
    })?;
    if length > 16 * 1024 * 1024 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Observed {
        method,
        target,
        headers,
        body,
    })
}

fn pointer(data: &[u8]) -> Pointer {
    let oid = Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Pointer::new(oid, data.len() as u64).unwrap()
}

fn store() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let store = Store::new(temp.path().join("objects"));
    (temp, store)
}

fn batch(pointer: &Pointer, actions: Value) -> Value {
    json!({
        "transfer": "basic",
        "hash_algo": "sha256",
        "objects": [{ "oid": pointer.oid, "size": pointer.size, "actions": actions }]
    })
}

fn lock(id: &str, path: &str) -> Value {
    json!({ "id": id, "path": path, "locked_at": "2026-11-08T12:00:00Z", "owner": { "name": "Fixture user" } })
}

#[test]
fn downloads_preserve_action_headers_without_forwarding_repository_credentials() {
    let data = b"verified binary game asset";
    let pointer = pointer(data);
    for action_auth in [None, Some("Bearer storage-only")] {
        let storage = Server::new(|_| vec![Reply::bytes(data.to_vec())]);
        let server = Server::new(|_| {
            let mut headers = json!({ "X-Signed-Header": "storage-signature" });
            if let Some(value) = action_auth {
                headers["Authorization"] = json!(value);
            }
            vec![Reply::json(
                200,
                batch(
                    &pointer,
                    json!({
                        "download": { "href": format!("{}/asset?signature=private-query", storage.base), "header": headers }
                    }),
                ),
            )]
        });
        let (_temp, store) = store();
        server.client().download(&store, &pointer).unwrap();
        store.verify(&pointer).unwrap();
        assert_eq!(fs::read(store.path(&pointer).unwrap()).unwrap(), data);
        let api = server.requests();
        assert_eq!(api[0].headers["authorization"], "Bearer repository-secret");
        assert_eq!(api[0].method, "POST");
        assert_eq!(api[0].target, "/repo.git/info/lfs/objects/batch");
        let request: Value = serde_json::from_slice(&api[0].body).unwrap();
        assert_eq!(request["operation"], "download");
        assert_eq!(request["transfers"], json!(["basic"]));
        let transferred = storage.requests();
        assert_eq!(
            transferred[0]
                .headers
                .get("authorization")
                .map(String::as_str),
            action_auth
        );
        assert_eq!(
            transferred[0].headers["x-signed-header"],
            "storage-signature"
        );
    }
}

#[test]
fn same_origin_download_uses_repository_authorization() {
    let data = b"same-origin asset";
    let pointer = pointer(data);
    let server = Server::new(|base| {
        vec![
            Reply::json(
                200,
                batch(
                    &pointer,
                    json!({ "download": { "href": format!("{base}/asset") } }),
                ),
            ),
            Reply::bytes(data.to_vec()),
        ]
    });
    let (_temp, store) = store();
    server.client().download(&store, &pointer).unwrap();
    assert_eq!(
        server.requests()[1].headers["authorization"],
        "Bearer repository-secret"
    );
}

#[test]
fn redirects_never_forward_repository_or_action_credentials() {
    let pointer = pointer(b"asset");
    let sink = Server::new(|_| vec![Reply::bytes(b"asset".to_vec())]);
    let server = Server::new(|_| vec![Reply::redirect(&format!("{}/leak", sink.base))]);
    let (_temp, cache) = store();
    let error = server
        .client()
        .download(&cache, &pointer)
        .unwrap_err()
        .to_string();
    assert!(error.contains("HTTP 307"));
    assert!(sink.requests().is_empty());

    let storage = Server::new(|_| {
        vec![Reply::redirect(&format!(
            "{}/leak?private-query",
            sink.base
        ))]
    });
    let server = Server::new(|_| {
        vec![Reply::json(
            200,
            batch(
                &pointer,
                json!({
                    "download": { "href": format!("{}/redirect", storage.base), "header": { "Authorization": "Bearer action-secret" } }
                }),
            ),
        )]
    });
    let error = server
        .client()
        .download(&cache, &pointer)
        .unwrap_err()
        .to_string();
    assert!(error.contains("HTTP 307"));
    assert!(!error.contains("secret") && !error.contains("private-query"));
    assert!(sink.requests().is_empty());
}

#[test]
fn corruption_truncation_and_unbounded_overlength_responses_are_not_published() {
    let expected = b"expected";
    let pointer = pointer(expected);
    let replies = [
        Reply::bytes(b"corrupt!".to_vec()),
        Reply::bytes(b"short".to_vec()).without_length(),
        Reply::bytes(b"expectedEXTRA".to_vec()).without_length(),
        Reply::bytes(b"expectedEXTRA".to_vec()),
    ];
    for reply in replies {
        let storage = Server::new(|_| vec![reply]);
        let server = Server::new(|_| {
            vec![Reply::json(
                200,
                batch(
                    &pointer,
                    json!({
                        "download": { "href": format!("{}/asset", storage.base) }
                    }),
                ),
            )]
        });
        let (_temp, cache) = store();
        assert!(server.client().download(&cache, &pointer).is_err());
        assert!(!cache.path(&pointer).unwrap().exists());
    }
}

#[test]
fn batch_identity_algorithm_adapter_and_object_errors_are_checked() {
    let pointer = pointer(b"asset");
    let sink = Server::new(|_| vec![Reply::bytes(b"asset".to_vec())]);
    let good = batch(
        &pointer,
        json!({ "download": { "href": format!("{}/asset", sink.base) } }),
    );
    let mut wrong_oid = good.clone();
    wrong_oid["objects"][0]["oid"] = json!("0".repeat(64));
    let mut wrong_size = good.clone();
    wrong_size["objects"][0]["size"] = json!(999);
    let mut wrong_hash = good.clone();
    wrong_hash["hash_algo"] = json!("sha1");
    let mut wrong_adapter = good.clone();
    wrong_adapter["transfer"] = json!("unadvertised");
    let mut object_error = good.clone();
    object_error["objects"][0]["error"] =
        json!({ "code": 403, "message": "Bearer secret https://server/?private-query" });
    let mut extra_object = good.clone();
    extra_object["objects"]
        .as_array_mut()
        .unwrap()
        .push(good["objects"][0].clone());
    let missing_download = batch(&pointer, json!({}));
    for response in [
        wrong_oid,
        wrong_size,
        wrong_hash,
        wrong_adapter,
        object_error,
        extra_object,
        missing_download,
    ] {
        let server = Server::new(|_| vec![Reply::json(200, response)]);
        let (_temp, cache) = store();
        let error = format!(
            "{:#}",
            server.client().download(&cache, &pointer).unwrap_err()
        );
        assert!(!error.contains("Bearer secret") && !error.contains("private-query"));
        assert!(sink.requests().is_empty());
    }
}

#[test]
fn json_response_limit_applies_without_a_content_length() {
    let server = Server::new(|_| vec![Reply::bytes(vec![b' '; 1024 * 1024 + 1]).without_length()]);
    let (_temp, cache) = store();
    let error = server
        .client()
        .download(&cache, &pointer(b"asset"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("JSON limit"), "{error}");
}

#[test]
fn url_and_header_rejections_do_not_echo_secrets() {
    for (endpoint, insecure) in [
        ("http://example.com/repo", false),
        ("http://example.com/repo", true),
        ("http://localhost/repo", true),
        ("http://127.0.0.1/repo", false),
        ("https://user:private-secret@example.com/repo", false),
        ("file:///private-secret", false),
        ("https://example.com/repo?private-secret", false),
        ("https://example.com/repo#private-secret", false),
    ] {
        let error = Client::new(endpoint, None, insecure)
            .err()
            .unwrap()
            .to_string();
        assert!(!error.contains("private-secret"));
    }
    assert!(Client::new("http://127.0.0.1/repo", None, true).is_ok());
    assert!(Client::new("http://[::1]/repo", None, true).is_ok());
    assert!(
        Client::new(
            "https://example.com/repo",
            Some("Bearer private-secret\r\nInjected: yes".into()),
            false
        )
        .is_err()
    );

    let pointer = pointer(b"asset");
    let sink = Server::new(|_| vec![Reply::bytes(b"asset".to_vec())]);
    for headers in [
        json!({ "X-Invalid": "private-secret\r\nInjected: yes" }),
        json!({ "Host": "private-secret.example" }),
        json!({ "Authorization": "private-secret", "authorization": "other" }),
        json!({ "Content-Length": "1000" }),
        json!({ "Transfer-Encoding": "chunked" }),
    ] {
        let server = Server::new(|_| {
            vec![Reply::json(
                200,
                batch(
                    &pointer,
                    json!({
                        "download": { "href": format!("{}/asset?private-secret", sink.base), "header": headers }
                    }),
                ),
            )]
        });
        let (_temp, cache) = store();
        let error = format!(
            "{:#}",
            server.client().download(&cache, &pointer).unwrap_err()
        );
        assert!(!error.contains("private-secret"));
        assert!(sink.requests().is_empty());
    }
}

#[test]
fn upload_streams_the_file_and_verifies_with_separate_action_credentials() {
    let data: Vec<u8> = (0..1024 * 1024 + 19)
        .map(|index| (index % 251) as u8)
        .collect();
    let pointer = pointer(&data);
    let storage = Server::new(|_| vec![Reply::bytes(Vec::new())]);
    let verification = Server::new(|_| vec![Reply::bytes(Vec::new())]);
    let server = Server::new(|_| {
        vec![Reply::json(
            200,
            batch(
                &pointer,
                json!({
                    "upload": {
                        "href": format!("{}/object?private-upload", storage.base),
                        "header": { "Authorization": "Bearer upload-only", "X-Object": "signed-value" }
                    },
                    "verify": {
                        "href": format!("{}/verify?private-verify", verification.base),
                        "header": { "Authorization": "Bearer verify-only" }
                    }
                }),
            ),
        )]
    });
    let (_temp, cache) = store();
    cache.ingest(data.as_slice(), Some(&pointer)).unwrap();
    server.client().upload(&cache, &pointer).unwrap();
    let requests = storage.requests();
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(requests[0].body, data);
    assert_eq!(
        requests[0].headers["content-length"],
        pointer.size.to_string()
    );
    assert_eq!(requests[0].headers["authorization"], "Bearer upload-only");
    assert_eq!(requests[0].headers["x-object"], "signed-value");
    let verify = verification.requests();
    assert_eq!(verify[0].method, "POST");
    assert_eq!(verify[0].headers["authorization"], "Bearer verify-only");
    assert_eq!(
        serde_json::from_slice::<Value>(&verify[0].body).unwrap(),
        json!({ "oid": pointer.oid, "size": pointer.size })
    );
}

#[test]
fn existing_remote_objects_need_no_upload_and_local_objects_must_be_verified() {
    let data = b"already exists remotely";
    let pointer = pointer(data);
    let server = Server::new(|_| {
        vec![Reply::json(
            200,
            json!({
                "objects": [{ "oid": pointer.oid, "size": pointer.size }]
            }),
        )]
    });
    let (_temp, cache) = store();
    assert!(server.client().upload(&cache, &pointer).is_err());
    assert!(server.requests().is_empty());
    cache.ingest(data.as_slice(), Some(&pointer)).unwrap();
    server.client().upload(&cache, &pointer).unwrap();
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn upload_rejects_failed_http_and_does_not_run_verification() {
    let data = b"asset";
    let pointer = pointer(data);
    let sink = Server::new(|_| vec![Reply::bytes(Vec::new())]);
    let storage = Server::new(|_| vec![Reply::json(403, json!({ "message": "private-secret" }))]);
    let server = Server::new(|_| {
        vec![Reply::json(
            200,
            batch(
                &pointer,
                json!({
                    "upload": { "href": format!("{}/object?private-secret", storage.base) },
                    "verify": { "href": format!("{}/verify", sink.base) }
                }),
            ),
        )]
    });
    let (_temp, cache) = store();
    cache.ingest(data.as_slice(), Some(&pointer)).unwrap();
    let error = format!(
        "{:#}",
        server.client().upload(&cache, &pointer).unwrap_err()
    );
    assert!(error.contains("HTTP 403"));
    assert!(!error.contains("private-secret"));
    assert!(sink.requests().is_empty());
}

#[test]
fn locking_roundtrip_handles_pages_and_encodes_opaque_ids() {
    let ours = lock("asset/one", "Content/Hero.uasset");
    let theirs = lock("asset-two", "Content/World.umap");
    let cursor = "next+ /?&";
    let server = Server::new(|_| {
        vec![
            Reply::json(201, json!({ "lock": ours })),
            Reply::json(200, json!({ "locks": [ours], "next_cursor": cursor })),
            Reply::json(200, json!({ "locks": [theirs] })),
            Reply::json(
                200,
                json!({ "ours": [ours], "theirs": [], "next_cursor": cursor }),
            ),
            Reply::json(
                200,
                json!({ "ours": [], "theirs": [theirs], "next_cursor": "" }),
            ),
            Reply::json(200, json!({ "lock": ours })),
        ]
    });
    let client = server.client();
    let acquired = client.lock("Content/Hero.uasset").unwrap();
    assert_eq!(acquired.id, "asset/one");
    let listed = client.list_locks().unwrap();
    assert_eq!(listed.len(), 2);
    let verified = client.verify_locks().unwrap();
    assert_eq!(verified.ours.len(), 1);
    assert_eq!(verified.theirs[0].path, "Content/World.umap");
    assert_eq!(client.unlock(&acquired.id, false).unwrap(), acquired);
    let requests = server.requests();
    assert_eq!(requests.len(), 6);
    assert!(
        requests
            .iter()
            .all(|request| request.headers["authorization"] == "Bearer repository-secret")
    );
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[0].body).unwrap(),
        json!({ "path": "Content/Hero.uasset" })
    );
    assert_eq!(requests[1].method, "GET");
    let second_page = url::Url::parse(&format!("{}{}", server.base, requests[2].target)).unwrap();
    assert_eq!(
        second_page
            .query_pairs()
            .find(|(name, _)| name == "cursor")
            .unwrap()
            .1,
        cursor
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[4].body).unwrap()["cursor"],
        cursor
    );
    assert_eq!(
        requests[5].target,
        "/repo.git/info/lfs/locks/asset%2Fone/unlock"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[5].body).unwrap(),
        json!({ "force": false })
    );
}

#[test]
fn lock_protocol_rejects_cycles_conflicts_and_mismatched_mutation_responses() {
    let item = lock("one", "Content/Hero.uasset");
    let server = Server::new(|_| {
        vec![
            Reply::json(200, json!({ "locks": [item], "next_cursor": "cycle" })),
            Reply::json(200, json!({ "locks": [item], "next_cursor": "cycle" })),
            Reply::json(409, json!({ "message": "private-secret", "lock": item })),
            Reply::json(201, json!({ "lock": lock("two", "Content/Other.uasset") })),
            Reply::json(200, json!({ "lock": item })),
        ]
    });
    let client = server.client();
    assert!(
        client
            .list_locks()
            .unwrap_err()
            .to_string()
            .contains("repeated")
    );
    let conflict = client.lock("Content/Hero.uasset").unwrap_err().to_string();
    assert!(conflict.contains("HTTP 409") && !conflict.contains("private-secret"));
    assert!(
        client
            .lock("Content/Hero.uasset")
            .unwrap_err()
            .to_string()
            .contains("different path")
    );
    assert!(
        client
            .unlock("two", true)
            .unwrap_err()
            .to_string()
            .contains("different lock")
    );
    for path in [
        "",
        "/absolute",
        "../outside",
        "a/../b",
        "a//b",
        "C:\\outside",
        "a\\b",
    ] {
        assert!(client.lock(path).is_err());
    }
    assert_eq!(server.requests().len(), 5);
}
