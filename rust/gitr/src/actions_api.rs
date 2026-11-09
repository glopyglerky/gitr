//! Optional local HTTP adapter over the existing redb/Wasmi action service.
//! No replacement datastore, raw query interface, session authority or MCP claim.
use super::*;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State,
    http::{Request as HttpRequest, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const MAX_REQUEST: usize = 2 * MAX_BYTES + 16 * 1024;
const REQUESTS: usize = 8;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Install {
        wasm_hex: String,
    },
    Submit {
        job: String,
        module: String,
        revision: String,
        path: String,
    },
    Status {
        job: String,
    },
    Cancel {
        job: String,
    },
    Run {
        job: String,
    },
    Artifact {
        job: String,
    },
}

impl Operation {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Install { wasm_hex } => {
                decode(wasm_hex)?;
            }
            Self::Submit {
                job,
                module,
                revision,
                path,
            } => {
                key(job)?;
                oid(module)?;
                ensure!(
                    !revision.is_empty() && revision.len() <= 4096,
                    "invalid revision"
                );
                ensure!(
                    !path.is_empty()
                        && path.len() <= 4096
                        && !path.contains(['\0', '\\'])
                        && !path
                            .split('/')
                            .any(|p| p.is_empty() || p == "." || p == ".."),
                    "invalid path"
                );
            }
            Self::Status { job }
            | Self::Cancel { job }
            | Self::Run { job }
            | Self::Artifact { job } => key(job)?,
        }
        Ok(())
    }
}

#[derive(Clone)]
struct Api {
    db: Arc<ActionDb>,
    discovery_root: PathBuf,
    repository: String,
    authorization: Arc<Vec<u8>>,
    permits: Arc<Semaphore>,
}

impl Api {
    fn perform(&self, operation: Operation) -> Result<Value> {
        match operation {
            Operation::Install { wasm_hex } => Ok(
                json!({"module_sha256":self.db.install(&decode(&wasm_hex)?)?, "profile":PROFILE}),
            ),
            Operation::Submit {
                job,
                module,
                revision,
                path,
            } => {
                // The caller supplies a revision/path, not a forged commit/blob
                // relation. Resolve immutable Git bytes with the same native code
                // used by the CLI, without hooks or a worktree checkout.
                let repo = crate::git::discover_at(&self.discovery_root)?;
                let input = SnapshotInput::from_git(&repo, &revision, &path)?;
                Ok(serde_json::to_value(self.db.submit(&job, &module, input)?)?)
            }
            Operation::Status { job } => Ok(serde_json::to_value(self.db.get(&job)?)?),
            Operation::Cancel { job } => Ok(serde_json::to_value(self.db.cancel(&job)?)?),
            Operation::Run { job } => Ok(serde_json::to_value(self.db.run_once(&job)?)?),
            Operation::Artifact { job } => {
                let bytes = self.db.artifact(&job)?;
                Ok(json!({"sha256":digest(&bytes), "hex":encode(&bytes)}))
            }
        }
    }
}

fn encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn decode(value: &str) -> Result<Vec<u8>> {
    ensure!(
        value.len() <= MAX_BYTES * 2
            && value.len().is_multiple_of(2)
            && value
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "invalid bounded lowercase hex"
    );
    let digit = |c: u8| if c <= b'9' { c - b'0' } else { c - b'a' + 10 };
    Ok(value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| digit(p[0]) * 16 + digit(p[1]))
        .collect())
}
fn authorized(expected: &[u8], provided: &[u8]) -> bool {
    expected.len() == provided.len() && bool::from(expected.ct_eq(provided))
}
fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({"error":code}))).into_response()
}

/// Disconnecting the HTTP waiter must not release admission while blocking work
/// is still running. The work, not its JoinHandle or request, owns the permit.
async fn offload<T: Send + 'static>(
    permit: OwnedSemaphorePermit,
    work: impl FnOnce() -> T + Send + 'static,
) -> std::result::Result<T, tokio::task::JoinError> {
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await
}

async fn handle(State(api): State<Api>, request: HttpRequest<Body>) -> Response {
    let headers = request.headers();
    let provided = headers
        .get(header::AUTHORIZATION)
        .map(|v| v.as_bytes())
        .unwrap_or_default();
    if !authorized(api.authorization.as_slice(), provided) {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    if headers.contains_key(header::ORIGIN)
        || headers
            .get("x-gitr-repository")
            .and_then(|v| v.to_str().ok())
            != Some(api.repository.as_str())
    {
        return error(StatusCode::FORBIDDEN, "repository_scope");
    }
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        != Some("application/json")
    {
        return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "content_type");
    }
    let Ok(permit) = api.permits.clone().try_acquire_owned() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "busy");
    };
    let bytes = match tokio::time::timeout(
        Duration::from_secs(10),
        to_bytes(request.into_body(), MAX_REQUEST),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        _ => return error(StatusCode::PAYLOAD_TOO_LARGE, "body_limit_or_timeout"),
    };
    let operation: Operation = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    if operation.validate().is_err() {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    match offload(permit, move || api.perform(operation)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        // Do not expose repository paths, module contents or underlying errors.
        // This initial adapter deliberately does not guess typed conflict versus
        // IO classifications from anyhow message text.
        Ok(Err(_)) => error(StatusCode::UNPROCESSABLE_ENTITY, "operation_failed"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "worker_failed"),
    }
}

pub(super) fn serve(repo: &gix::Repository, listen: SocketAddr, repository: &str) -> Result<()> {
    ensure!(
        listen.ip() == IpAddr::V4(Ipv4Addr::LOCALHOST),
        "action API binds only literal IPv4 loopback"
    );
    key(repository)?;
    let token = std::env::var("GITR_API_TOKEN").context(
        "GITR_API_TOKEN must contain a generated 64-character lowercase hexadecimal secret",
    )?;
    oid(&token).context("invalid API token format")?;
    let root = repo.workdir().unwrap_or_else(|| repo.git_dir());
    let api = Api {
        db: Arc::new(ActionDb::open_for_cli(repo, true)?),
        discovery_root: fs::canonicalize(root)?,
        repository: repository.into(),
        authorization: Arc::new(format!("Bearer {token}").into_bytes()),
        permits: Arc::new(Semaphore::new(REQUESTS)),
    };
    tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2)
        .max_blocking_threads(REQUESTS).thread_stack_size(4 * 1024 * 1024).build()?.block_on(async move {
            let listener = tokio::net::TcpListener::bind(listen).await?;
            let address = listener.local_addr()?;
            print_json(&json!({"endpoint":format!("http://{address}/v1/actions"), "repository":repository, "backend":"redb", "schema":1}))?;
            io::stdout().flush()?;
            axum::serve(listener, Router::new().route("/v1/actions", post(handle)).with_state(api)).await?;
            Ok(())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_hex_preserves_binary_and_rejects_invalid_inputs() {
        let bytes = [0, 1, 127, 128, 255];
        assert_eq!(decode(&encode(&bytes)).unwrap(), bytes);
        for value in ["0", "GG", "FF", "00 ", "../secret"] {
            assert!(decode(value).is_err());
        }
        assert!(decode(&"0".repeat(MAX_BYTES * 2 + 2)).is_err());
    }

    #[test]
    fn control_rejects_arbitrary_paths_queries_and_worker_receipts() {
        for request in [
            r#"{"operation":"status","job":"job","database":"/etc/passwd"}"#,
            r#"{"operation":"query","sql":"delete jobs"}"#,
            r#"{"operation":"finish","job":"job","attempt":1,"output_hex":"00"}"#,
        ] {
            assert!(serde_json::from_str::<Operation>(request).is_err());
        }
        assert!(
            Operation::Status {
                job: "../job".into()
            }
            .validate()
            .is_err()
        );
        assert!(!authorized(b"Bearer secret", b"Bearer secreu"));
        assert!(!authorized(b"Bearer secret", b""));
        assert!(authorized(b"Bearer secret", b"Bearer secret"));
    }

    #[test]
    fn a_disconnected_waiter_cannot_release_a_running_workers_permit() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let semaphore = Arc::new(Semaphore::new(1));
                let permit = semaphore.clone().try_acquire_owned().unwrap();
                let (started, ready) = tokio::sync::oneshot::channel();
                let (release, wait) = std::sync::mpsc::channel();
                let waiter = tokio::spawn(offload(permit, move || {
                    started.send(()).unwrap();
                    wait.recv().unwrap();
                }));
                ready.await.unwrap();
                waiter.abort();
                assert!(waiter.await.unwrap_err().is_cancelled());
                assert!(semaphore.clone().try_acquire_owned().is_err());
                release.send(()).unwrap();
                let permit =
                    tokio::time::timeout(Duration::from_secs(3), semaphore.acquire_owned())
                        .await
                        .unwrap()
                        .unwrap();
                drop(permit);
            });
    }
}
