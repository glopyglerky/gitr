//! Explicit opt-in transport to the independent one-owner storage service.
//! This is a child module of actions so the existing sandbox is reused exactly.
use super::*;
use serde_json::{Value, json};
use ureq::{Agent, http::{HeaderValue, header}};
use url::Url;

#[derive(Debug, Subcommand)]
pub enum Command {
    Install { module: PathBuf },
    Submit { job: String, module: String, #[arg(long, default_value = "HEAD")] revision: String, #[arg(long)] path: String },
    Run { job: String },
    Status { job: String },
    Cancel { job: String },
    Artifact { job: String },
}
struct Client { endpoint: Url, authorization: HeaderValue, repository: String, agent: Agent }
impl Client {
    fn new(endpoint: &str, repository: &str) -> Result<Self> {
        let endpoint = Url::parse(endpoint).context("invalid storage service endpoint")?;
        ensure!(endpoint.scheme() == "http" && endpoint.host_str() == Some("127.0.0.1")
            && endpoint.username().is_empty() && endpoint.password().is_none() && endpoint.query().is_none()
            && endpoint.fragment().is_none() && endpoint.path() == "/v1/action", "storage service must be explicit loopback /v1/action endpoint");
        key(repository)?;
        let token = std::env::var("GITR_STORE_TOKEN").context("GITR_STORE_TOKEN is required")?;
        oid(&token)?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))?;
        authorization.set_sensitive(true);
        let agent = Agent::config_builder().max_redirects(0).http_status_as_error(false).proxy(None)
            .timeout_global(Some(Duration::from_secs(30))).build().into();
        Ok(Self { endpoint, authorization, repository: repository.into(), agent })
    }
    fn call(&self, body: Value) -> Result<Value> {
        let payload = serde_json::to_vec(&body)?;
        ensure!(payload.len() <= 2 * MAX_BYTES + 32 * 1024, "storage request too large");
        let mut response = self.agent.post(self.endpoint.as_str())
            .header(header::AUTHORIZATION, self.authorization.clone())
            .header("x-gitr-repository", self.repository.as_str())
            .header(header::CONTENT_TYPE, "application/json")
            .send(payload.as_slice()).map_err(|_| anyhow::anyhow!("storage service transport failed"))?;
        ensure!(response.status().as_u16() == 200, "storage service returned HTTP {}", response.status().as_u16());
        let bytes = bounded_read(response.body_mut().as_reader(), 2 * MAX_BYTES + 32 * 1024)?;
        Ok(serde_json::from_slice(&bytes).context("invalid storage service response")?)
    }
    fn job(&self, operation: &str, id: &str) -> Result<Job> {
        key(id)?;
        let job: Job = serde_json::from_value(self.call(json!({"operation": operation, "job": id}))?)?;
        ensure!(job.id == id && job.spec.profile == PROFILE, "storage service returned a mismatched job");
        Ok(job)
    }
    fn object(&self, id: &str) -> Result<Vec<u8>> {
        oid(id)?;
        let value = self.call(json!({"operation":"object", "sha256":id}))?;
        let bytes = decode(value["hex"].as_str().context("missing object bytes")?)?;
        ensure!(digest(&bytes) == id, "storage object integrity mismatch");
        Ok(bytes)
    }
    fn run_once(&self, id: &str) -> Result<Job> {
        key(id)?;
        let response = self.call(json!({"operation":"claim", "job": id}))?;
        let claim = response.get("claim").context("missing claim response")?;
        if claim.is_null() { return self.job("status", id); }
        let claim: Job = serde_json::from_value(claim.clone())?;
        ensure!(claim.id == id && claim.spec.profile == PROFILE && claim.status == Status::Running && claim.attempt > 0, "invalid claim response");
        let module = self.object(&claim.spec.module_sha256)?;
        let input = self.object(&claim.spec.input_sha256)?;
        // Same capability-denying sandbox as local actions; no database handle
        // or writer transaction exists in this process during guest execution.
        let (exit_code, output_hex) = match execute(&module, input) {
            Ok((0, bytes)) => (Some(0), encode(&bytes)),
            Ok((code, _)) => (Some(code), String::new()),
            Err(_) => (None, String::new()),
        };
        let finished: Job = serde_json::from_value(self.call(json!({"operation":"finish", "job":id,
            "attempt":claim.attempt, "exit_code":exit_code, "output_hex":output_hex}))?)?;
        ensure!(finished.id == id && finished.spec == claim.spec && finished.attempt == claim.attempt
            && finished.exit_code == exit_code, "mismatched action receipt");
        Ok(finished)
    }
}
fn encode(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }
fn decode(value: &str) -> Result<Vec<u8>> {
    ensure!(value.len() <= MAX_BYTES * 2 && value.len().is_multiple_of(2)
        && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)), "invalid object hex");
    let digit = |c: u8| if c <= b'9' { c - b'0' } else { c - b'a' + 10 };
    Ok(value.as_bytes().chunks_exact(2).map(|p| digit(p[0]) * 16 + digit(p[1])).collect())
}
pub fn run(repo: &gix::Repository, endpoint: &str, repository: &str, command: Command) -> Result<()> {
    let client = Client::new(endpoint, repository)?;
    match command {
        Command::Install { module } => {
            let bytes = bounded_read(fs::File::open(module)?, MAX_BYTES)?;
            prepare(&bytes)?;
            let result = client.call(json!({"operation":"install", "module_hex":encode(&bytes)}))?;
            ensure!(result["module_sha256"].as_str() == Some(digest(&bytes).as_str()) && result["profile"] == PROFILE, "module installation identity mismatch");
            print_json(&result)
        }
        Command::Submit { job, module, revision, path } => {
            key(&job)?; oid(&module)?;
            let input = SnapshotInput::from_git(repo, &revision, &path)?;
            let spec = Spec { profile: PROFILE.into(), module_sha256: module, commit_oid: input.commit,
                blob_oid: input.blob, path: input.path, input_sha256: digest(&input.bytes) };
            let result: Job = serde_json::from_value(client.call(json!({"operation":"submit", "job":job, "spec":spec, "input_hex":encode(&input.bytes)}))?)?;
            ensure!(result.id == job && result.spec == spec, "submission identity mismatch");
            print_json(&result)
        }
        Command::Run { job } => {
            let result = client.run_once(&job)?;
            print_json(&result)?;
            ensure!(result.status == Status::Passed, "action has not passed");
            Ok(())
        }
        Command::Status { job } => print_json(&client.job("status", &job)?),
        Command::Cancel { job } => print_json(&client.job("cancel", &job)?),
        Command::Artifact { job } => {
            let receipt = client.job("status", &job)?;
            ensure!(receipt.status == Status::Passed, "job has no successful artifact");
            let id = receipt.artifact_sha256.context("missing artifact identity")?;
            io::stdout().lock().write_all(&client.object(&id)?)?;
            Ok(())
        }
    }
}
