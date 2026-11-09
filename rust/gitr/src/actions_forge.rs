//! Repository-local forge records in the existing durable action redb database.
//! Rbit supplies validated domain values. No SQLite, second ref authority, caller-
//! fabricated check result or inferred multi-user identity is introduced here.
use super::*;
use rbit_core::{IssueKind, IssueState, IssueText, IssueTitle};
use rbit_core::issue::ReviewState;
use serde_json::{Value, json};
use std::collections::BTreeMap;

const THREADS: TableDefinition<u32, &[u8]> = TableDefinition::new("forge.threads.v1");
const EVENTS: TableDefinition<(u32, u64), &[u8]> = TableDefinition::new("forge.events.v1");
const RECEIPTS: TableDefinition<&str, &[u8]> = TableDefinition::new("forge.receipts.v1");
const MAX_RECORD: usize = 256 * 1024;
const MAX_REQUEST: usize = 128 * 1024;
const MAX_PAGE: u32 = 8;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// One typed, bounded JSON request on stdin; mutations require operation_id.
    Control,
    Show { number: u32 },
    List { #[arg(long, default_value_t = 0)] after: u32, #[arg(long, default_value_t = 8)] limit: u32 },
    Timeline { number: u32, #[arg(long, default_value_t = 0)] after: u64, #[arg(long, default_value_t = 8)] limit: u32 },
    /// Inspect exact-revision reviews/checks. This never authorizes or performs a merge.
    Check { number: u32 },
    /// Export only forge records in one snapshot; action objects are a separate export.
    Export,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequiredCheck {
    pub module_sha256: String,
    pub path: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub head_oid: String,
    pub decision: ReviewState,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Pull {
    pub base_oid: String,
    pub head_oid: String,
    pub required_checks: BTreeMap<String, RequiredCheck>,
    pub check_jobs: BTreeMap<String, String>,
    pub review: Option<Review>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Thread {
    pub schema: u32,
    pub number: u32,
    pub revision: u64,
    pub kind: IssueKind,
    pub state: IssueState,
    pub title: String,
    pub body: String,
    pub comment_count: u64,
    pub created_ms: u64,
    pub updated_ms: u64,
    /// A local authority label, NOT an authenticated human or independent reviewer.
    pub authority: String,
    pub pull: Option<Pull>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub number: u32,
    pub revision: u64,
    pub operation_id: String,
    pub request_sha256: String,
    pub at_ms: u64,
    pub authority: String,
    pub request: Request,
    pub resolved_head: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt { version: u32, request_sha256: String, result: Thread }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    CreateIssue { operation_id: String, title: String, body: String },
    CreatePull { operation_id: String, title: String, body: String, base: String, head: String, required_checks: BTreeMap<String, RequiredCheck> },
    Edit { operation_id: String, number: u32, expected_revision: u64, title: String, body: String },
    SetState { operation_id: String, number: u32, expected_revision: u64, state: IssueState },
    Comment { operation_id: String, number: u32, body: String },
    Retarget { operation_id: String, number: u32, expected_revision: u64, base: String, head: String },
    Review { operation_id: String, number: u32, expected_revision: u64, head_oid: String, decision: ReviewState },
    AttachCheck { operation_id: String, number: u32, expected_revision: u64, context: String, job: String },
    Get { number: u32 },
    List { after: u32, limit: u32 },
    Timeline { number: u32, after: u64, limit: u32 },
    Readiness { number: u32 },
}
impl Request {
    fn operation_id(&self) -> Option<&str> {
        match self {
            Self::CreateIssue { operation_id, .. } | Self::CreatePull { operation_id, .. }
            | Self::Edit { operation_id, .. } | Self::SetState { operation_id, .. }
            | Self::Comment { operation_id, .. } | Self::Retarget { operation_id, .. }
            | Self::Review { operation_id, .. } | Self::AttachCheck { operation_id, .. } => Some(operation_id),
            _ => None,
        }
    }
    fn number(&self) -> Option<u32> {
        match self {
            Self::Edit { number, .. } | Self::SetState { number, .. } | Self::Comment { number, .. }
            | Self::Retarget { number, .. } | Self::Review { number, .. } | Self::AttachCheck { number, .. }
            | Self::Get { number } | Self::Timeline { number, .. } | Self::Readiness { number } => Some(*number),
            _ => None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(serde_json::to_vec(self)?.len() <= MAX_REQUEST, "forge request exceeds limit");
        if let Some(id) = self.operation_id() { key(id)?; }
        if let Some(number) = self.number() { ensure!(number > 0, "invalid issue number"); }
        match self {
            Self::CreateIssue { title, body, .. } | Self::CreatePull { title, body, .. } | Self::Edit { title, body, .. } => {
                IssueTitle::parse(title)?; IssueText::body(body)?;
            }
            Self::Comment { body, .. } => { IssueText::comment(body)?; }
            Self::Review { head_oid, decision, .. } => {
                full_oid(head_oid)?;
                ensure!(matches!(decision, ReviewState::Approved | ReviewState::ChangesRequested | ReviewState::Commented), "unsupported review decision");
            }
            Self::AttachCheck { context, job, .. } => { key(context)?; key(job)?; }
            Self::List { limit, .. } | Self::Timeline { limit, .. } => ensure!((1..=MAX_PAGE).contains(limit), "invalid page limit"),
            _ => (),
        }
        match self {
            Self::CreatePull { base, head, required_checks, .. } => { revision(base)?; revision(head)?; checks(required_checks)?; }
            Self::Retarget { base, head, .. } => { revision(base)?; revision(head)?; }
            _ => (),
        }
        Ok(())
    }
}
fn revision(value: &str) -> Result<()> { ensure!(!value.is_empty() && value.len() <= 4096 && !value.contains('\0'), "invalid revision"); Ok(()) }
fn full_oid(value: &str) -> Result<()> {
    ensure!([40,64].contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "expected canonical full Git identity"); Ok(())
}
fn checks(checks: &BTreeMap<String, RequiredCheck>) -> Result<()> {
    ensure!(checks.len() <= 16, "too many required checks");
    for (context, check) in checks {
        key(context)?; oid(&check.module_sha256)?;
        ensure!(!check.path.is_empty() && check.path.len() <= 1024 && !check.path.contains(['\0','\\'])
            && !check.path.split('/').any(|p| p.is_empty() || p == "." || p == ".."), "invalid check input path");
    }
    Ok(())
}
fn resolve(repo: &gix::Repository, value: &str) -> Result<String> {
    revision(value)?;
    Ok(repo.rev_parse_single(value.as_bytes().as_bstr())?.object()?.peel_to_kind(gix::objs::Kind::Commit)?.id.to_string())
}
fn decode_thread(bytes: &[u8], number: u32) -> Result<Thread> {
    ensure!(bytes.len() <= MAX_RECORD, "oversized forge record");
    let thread: Thread = serde_json::from_slice(bytes)?;
    ensure!(thread.schema == 1 && number > 0 && thread.number == number && thread.revision > 0
        && thread.authority == "local-operator", "invalid forge record identity");
    ensure!(IssueTitle::parse(&thread.title)?.as_str() == thread.title && IssueText::body(&thread.body)?.as_str() == thread.body, "invalid stored text");
    ensure!((thread.kind == IssueKind::Pull) == thread.pull.is_some(), "inconsistent thread kind");
    if let Some(pull) = &thread.pull {
        full_oid(&pull.base_oid)?; full_oid(&pull.head_oid)?; checks(&pull.required_checks)?;
        ensure!(pull.base_oid.len() == pull.head_oid.len(), "mixed object formats");
        for (name, job) in &pull.check_jobs { ensure!(pull.required_checks.contains_key(name), "unknown linked check"); key(job)?; }
        if let Some(review) = &pull.review {
            ensure!(review.head_oid == pull.head_oid && matches!(review.decision, ReviewState::Approved | ReviewState::ChangesRequested | ReviewState::Commented), "stale or invalid stored review");
        }
    }
    Ok(thread)
}
fn read_thread(tx: &redb::WriteTransaction, number: u32) -> Result<Thread> {
    let table = tx.open_table(THREADS)?;
    let bytes = table.get(number)?.context("unknown forge thread")?;
    decode_thread(bytes.value(), number)
}
fn receipt(bytes: &[u8], expected: &str) -> Result<Thread> {
    ensure!(bytes.len() <= MAX_RECORD, "oversized operation receipt");
    let receipt: Receipt = serde_json::from_slice(bytes)?;
    ensure!(receipt.version == 1 && receipt.request_sha256 == expected, "operation key already belongs to a different request");
    decode_thread(&serde_json::to_vec(&receipt.result)?, receipt.result.number)
}
fn check_object(objects: &redb::ReadOnlyTable<&str, &[u8]>, id: &str) -> Result<()> {
    oid(id)?;
    let bytes = objects.get(id)?.context("missing check object")?;
    ensure!(bytes.value().len() <= MAX_BYTES && digest(bytes.value()) == id, "corrupt check object");
    Ok(())
}
fn bound_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= MAX_RECORD, "forge record exceeds limit");
    Ok(bytes)
}

impl ActionDb {
    fn forge_initialize(&self) -> Result<()> {
        {
            let tx = self.db.begin_read()?;
            let version = tx.open_table(META)?.get("forge-schema")?.map(|v| v.value());
            if let Some(version) = version { ensure!(version == 1, "unsupported forge schema"); return Ok(()); }
        }
        let mut tx = self.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        let version = tx.open_table(META)?.get("forge-schema")?.map(|v| v.value());
        ensure!(version.is_none() || version == Some(1), "unsupported forge schema");
        tx.open_table(THREADS)?; tx.open_table(EVENTS)?; tx.open_table(RECEIPTS)?;
        tx.open_table(META)?.insert("forge-schema", 1)?;
        tx.commit()?;
        Ok(())
    }

    /// Local-authority service shared by CLI and authenticated loopback API.
    /// The caller must already be authorized for this repository. Caller identity
    /// is deliberately not accepted from JSON and multi-user roles are not claimed.
    pub fn forge_control(&self, repo: &gix::Repository, request: Request) -> Result<Value> {
        request.validate()?;
        self.forge_initialize()?;
        if let Some(id) = request.operation_id() {
            let fingerprint = digest(&serde_json::to_vec(&request)?);
            // Idempotent replay precedes mutable ref resolution: a branch moving
            // after the first commit cannot change the original command receipt.
            {
                let tx = self.db.begin_read()?;
                if let Some(value) = tx.open_table(RECEIPTS)?.get(id)? {
                    return Ok(serde_json::to_value(receipt(value.value(), &fingerprint)?)?);
                }
            }
            let commits = match &request {
                Request::CreatePull { base, head, .. } | Request::Retarget { base, head, .. } => Some((resolve(repo, base)?, resolve(repo, head)?)),
                _ => None,
            };
            // Verify exact committed check provenance before taking a writer.
            let attachment = if let Request::AttachCheck { number, context, job, .. } = &request {
                let current = self.forge_get(*number)?;
                let pull = current.pull.context("not a change proposal")?;
                let check = pull.required_checks.get(context).context("check is not required")?;
                let input = SnapshotInput::from_git(repo, &pull.head_oid, &check.path)?;
                Some((self.get(job)?, input, check.clone()))
            } else { None };
            return Ok(serde_json::to_value(self.forge_mutate(&request, &fingerprint, commits, attachment)?)?);
        }
        match request {
            Request::Get { number } => Ok(serde_json::to_value(self.forge_get(number)?)?),
            Request::List { after, limit } => self.forge_list(after, limit),
            Request::Timeline { number, after, limit } => self.forge_timeline(number, after, limit),
            Request::Readiness { number } => self.forge_readiness(repo, number),
            _ => bail!("invalid forge operation"),
        }
    }

    fn forge_mutate(&self, request: &Request, fingerprint: &str, commits: Option<(String,String)>, attachment: Option<(Job,SnapshotInput,RequiredCheck)>) -> Result<Thread> {
        let id = request.operation_id().context("missing operation id")?;
        let mut tx = self.db.begin_write()?;
        tx.set_durability(Durability::Immediate)?;
        {
            if let Some(value) = tx.open_table(RECEIPTS)?.get(id)? { return receipt(value.value(), fingerprint); }
        }
        let now = now_ms()?;
        let mut thread = if let Some(number) = request.number() { read_thread(&tx, number)? } else {
            let previous = tx.open_table(META)?.get("forge-last-thread")?.map_or(0, |v| v.value());
            let number: u32 = previous.checked_add(1).context("thread number overflow")?.try_into()?;
            ensure!(tx.open_table(THREADS)?.get(number)?.is_none(), "forge number collision");
            tx.open_table(META)?.insert("forge-last-thread", u64::from(number))?;
            Thread { schema:1, number, revision:0, kind:IssueKind::Issue, state:IssueState::Open,
                title:String::new(), body:String::new(), comment_count:0, created_ms:now, updated_ms:now,
                authority:"local-operator".into(), pull:None }
        };
        let expected = match request {
            Request::Edit { expected_revision, .. } | Request::SetState { expected_revision, .. }
            | Request::Retarget { expected_revision, .. } | Request::Review { expected_revision, .. }
            | Request::AttachCheck { expected_revision, .. } => Some(*expected_revision), _ => None,
        };
        if let Some(expected) = expected { ensure!(expected > 0 && thread.revision == expected, "stale forge revision"); }
        match request {
            Request::CreateIssue { title, body, .. } | Request::Edit { title, body, .. } => {
                thread.title = IssueTitle::parse(title)?.into(); thread.body = IssueText::body(body)?.into();
            }
            Request::CreatePull { title, body, required_checks, .. } => {
                let (base_oid,head_oid) = commits.context("missing resolved commits")?;
                thread.kind = IssueKind::Pull;
                thread.title = IssueTitle::parse(title)?.into(); thread.body = IssueText::body(body)?.into();
                thread.pull = Some(Pull { base_oid, head_oid, required_checks:required_checks.clone(), check_jobs:BTreeMap::new(), review:None });
            }
            Request::SetState { state, .. } => thread.state = *state,
            Request::Comment { body, .. } => {
                IssueText::comment(body)?;
                thread.comment_count = thread.comment_count.checked_add(1).context("comment count overflow")?;
            }
            Request::Retarget { .. } => {
                let pull = thread.pull.as_mut().context("not a change proposal")?;
                let (base,head) = commits.context("missing resolved commits")?;
                pull.base_oid = base; pull.head_oid = head;
                pull.check_jobs.clear(); pull.review = None;
            }
            Request::Review { head_oid, decision, .. } => {
                ensure!(thread.state == IssueState::Open, "closed change proposal");
                let pull = thread.pull.as_mut().context("not a change proposal")?;
                ensure!(head_oid == &pull.head_oid, "review targets a stale head");
                pull.review = Some(Review { head_oid:head_oid.clone(), decision:*decision });
            }
            Request::AttachCheck { context, job, .. } => {
                ensure!(thread.state == IssueState::Open, "closed change proposal");
                let pull = thread.pull.as_mut().context("not a change proposal")?;
                let (observed,input,required) = attachment.context("missing verified input")?;
                ensure!(pull.required_checks.get(context) == Some(&required) && input.commit == pull.head_oid, "check policy/head changed");
                let current = read_job(&tx, job)?;
                ensure!(current.spec == observed.spec && current.spec.commit_oid == pull.head_oid
                    && current.spec.module_sha256 == required.module_sha256 && current.spec.path == required.path
                    && current.spec.blob_oid == input.blob && current.spec.input_sha256 == digest(&input.bytes), "check does not match required committed input");
                ensure!([PROFILE, workflow::MANAGED_PROFILE].contains(&current.spec.profile.as_str()), "unsupported check runtime");
                // A queued/running job is linkable, but only a verified Passed
                // receipt can later contribute to the advisory readiness view.
                pull.check_jobs.insert(context.clone(), job.clone());
            }
            _ => bail!("not a mutation"),
        }
        thread.revision = thread.revision.checked_add(1).context("forge revision overflow")?;
        thread.updated_ms = now;
        let bytes = bound_bytes(&thread)?;
        decode_thread(&bytes, thread.number)?;
        let event = Event { number:thread.number, revision:thread.revision, operation_id:id.into(),
            request_sha256:fingerprint.into(), at_ms:now, authority:"local-operator".into(), request:request.clone(),
            resolved_head:thread.pull.as_ref().map(|p| p.head_oid.clone()) };
        let event_bytes = bound_bytes(&event)?;
        let receipt_bytes = bound_bytes(&Receipt { version:1, request_sha256:fingerprint.into(), result:thread.clone() })?;
        tx.open_table(THREADS)?.insert(thread.number, bytes.as_slice())?;
        ensure!(tx.open_table(EVENTS)?.get((thread.number,thread.revision))?.is_none(), "event revision collision");
        tx.open_table(EVENTS)?.insert((thread.number,thread.revision), event_bytes.as_slice())?;
        tx.open_table(RECEIPTS)?.insert(id, receipt_bytes.as_slice())?;
        tx.commit()?;
        Ok(thread)
    }

    pub fn forge_get(&self, number: u32) -> Result<Thread> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(THREADS)?;
        let bytes = table.get(number)?.context("unknown forge thread")?;
        decode_thread(bytes.value(), number)
    }
    fn forge_list(&self, after: u32, limit: u32) -> Result<Value> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(THREADS)?;
        let mut items = Vec::new();
        if let Some(start) = after.checked_add(1) {
            for row in table.range(start..)?.take(limit as usize) {
                let (number,bytes) = row?;
                items.push(decode_thread(bytes.value(), number.value())?);
            }
        }
        let cursor = items.last().map(|v| v.number);
        Ok(json!({"items":items,"next_after":cursor}))
    }
    fn forge_timeline(&self, number: u32, after: u64, limit: u32) -> Result<Value> {
        let tx = self.db.begin_read()?;
        ensure!(tx.open_table(THREADS)?.get(number)?.is_some(), "unknown forge thread");
        let table = tx.open_table(EVENTS)?;
        let mut items = Vec::new();
        if let Some(start) = after.checked_add(1) {
            for row in table.range((number,start)..=(number,u64::MAX))?.take(limit as usize) {
                let (key,bytes) = row?;
                ensure!(bytes.value().len() <= MAX_RECORD, "oversized forge event");
                let event: Event = serde_json::from_slice(bytes.value())?;
                ensure!((event.number,event.revision) == key.value(), "forge event identity mismatch");
                items.push(event);
            }
        }
        let cursor = items.last().map(|v| v.revision);
        Ok(json!({"items":items,"next_after":cursor}))
    }

    fn forge_readiness(&self, repo: &gix::Repository, number: u32) -> Result<Value> {
        // Snapshot membership is immutable. Never turn a check on an old head,
        // wrong module or different file into a passing current-head status.
        let tx = self.db.begin_read()?;
        let table = tx.open_table(THREADS)?;
        let bytes = table.get(number)?.context("unknown forge thread")?;
        let thread = decode_thread(bytes.value(), number)?;
        let pull = thread.pull.as_ref().context("not a change proposal")?;
        let jobs = tx.open_table(JOBS)?;
        let objects = tx.open_table(OBJECTS)?;
        let mut statuses = Vec::new();
        let mut details = Vec::new();
        for (name,required) in &pull.required_checks {
            let job_id = pull.check_jobs.get(name);
            let (state, detail) = if let Some(id) = job_id {
                let check = (|| -> Result<Job> {
                    let bytes = jobs.get(id.as_str())?.context("missing linked action")?;
                    ensure!(bytes.value().len() <= 64 * 1024, "oversized action receipt");
                    let job: Job = serde_json::from_slice(bytes.value())?;
                    let input = SnapshotInput::from_git(repo, &pull.head_oid, &required.path)?;
                    ensure!(job.id == *id && job.spec.commit_oid == pull.head_oid && job.spec.blob_oid == input.blob
                        && job.spec.path == required.path && job.spec.module_sha256 == required.module_sha256
                        && job.spec.input_sha256 == digest(&input.bytes)
                        && [PROFILE,workflow::MANAGED_PROFILE].contains(&job.spec.profile.as_str()), "check provenance mismatch");
                    check_object(&objects, &job.spec.module_sha256)?;
                    check_object(&objects, &job.spec.input_sha256)?;
                    if job.status == Status::Passed {
                        ensure!(job.exit_code == Some(0) && job.lease_until_ms.is_none(), "inconsistent successful receipt");
                        check_object(&objects, job.artifact_sha256.as_deref().context("missing successful artifact")?)?;
                    }
                    Ok(job)
                })();
                match check {
                    Ok(job) => match job.status {
                        Status::Passed => (rbit_core::status::StatusState::Success,"verified_passed"),
                        Status::Queued | Status::Running => (rbit_core::status::StatusState::Pending,"not_completed"),
                        Status::Failed | Status::Cancelled => (rbit_core::status::StatusState::Failure,"failed_or_cancelled"),
                    },
                    Err(_) => (rbit_core::status::StatusState::Error,"unverifiable_receipt"),
                }
            } else { (rbit_core::status::StatusState::Pending,"missing_check") };
            statuses.push(rbit_core::status::CommitStatus {
                id:rbit_core::status::StatusId(1), repo_id:rbit_core::RepoId(1), sha:pull.head_oid.clone(), state,
                context:rbit_core::status::StatusContext::parse(name)?, description:None,target_url:None,
                creator:None,source:rbit_core::status::StatusSource::Actions,
                created_at:rbit_core::Timestamp(0),updated_at:rbit_core::Timestamp(0),
            });
            details.push(json!({"context":name,"job":job_id,"state":state,"detail":detail}));
        }
        let required: Vec<_> = pull.required_checks.keys().collect();
        let summary = rbit_core::status::required_checks(&required, &statuses);
        let approved = pull.review.as_ref().is_some_and(|r| r.head_oid == pull.head_oid && r.decision == ReviewState::Approved);
        Ok(json!({"number":number,"revision":thread.revision,"head_oid":pull.head_oid,
            "state":thread.state,"required":summary.required,"passing":summary.passing,
            "failing":summary.failing,"waiting":summary.waiting(),"checks":details,
            "local_review_approved":approved,"checks_configured":summary.required > 0,
            "locally_ready":thread.state == IssueState::Open && approved && summary.required > 0 && summary.all_pass(),
            "authorizes_merge":false,"scope":"local-operator advisory; not branch protection or independent approval"}))
    }

    /// Forge-only export. Exact records are length-framed into a footer digest.
    /// This is not import/restore or an atomic combined forge+action backup.
    pub fn forge_export(&self, out: &mut impl Write) -> Result<()> {
        self.forge_initialize()?;
        let tx = self.db.begin_read()?;
        let mut hash = Sha256::new();
        let mut counts = [0u64;3];
        fn emit(out: &mut impl Write, value: &impl Serialize) -> Result<()> { serde_json::to_writer(&mut *out,value)?; out.write_all(b"\n")?; Ok(()) }
        fn row(out: &mut impl Write, hash: &mut Sha256, kind: &str, key: &[u8], value: &[u8]) -> Result<()> {
            hash.update(kind.as_bytes()); hash.update([0]); hash.update((key.len() as u64).to_be_bytes()); hash.update(key);
            hash.update((value.len() as u64).to_be_bytes()); hash.update(value);
            let hex = |bytes: &[u8]| -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() };
            emit(out,&json!({"kind":kind,"key_hex":hex(key),"data_hex":hex(value)}))
        }
        emit(out,&json!({"format":"gitr-forge-export-v1","scope":"forge-only; action objects and Git history excluded"}))?;
        let last = tx.open_table(META)?.get("forge-last-thread")?.map_or(0, |v| v.value());
        row(out,&mut hash,"meta",b"forge-last-thread",&last.to_be_bytes())?;
        for entry in tx.open_table(THREADS)?.iter()? {
            let (number,bytes) = entry?; decode_thread(bytes.value(),number.value())?;
            row(out,&mut hash,"thread",&number.value().to_be_bytes(),bytes.value())?; counts[0] += 1;
        }
        for entry in tx.open_table(EVENTS)?.iter()? {
            let (key,bytes) = entry?; ensure!(bytes.value().len() <= MAX_RECORD,"oversized forge event");
            let event: Event = serde_json::from_slice(bytes.value())?;
            ensure!((event.number,event.revision) == key.value(),"forge event identity mismatch");
            let mut raw = event.number.to_be_bytes().to_vec(); raw.extend_from_slice(&event.revision.to_be_bytes());
            row(out,&mut hash,"event",&raw,bytes.value())?; counts[1] += 1;
        }
        for entry in tx.open_table(RECEIPTS)?.iter()? {
            let (id,bytes) = entry?; key(id.value())?; ensure!(bytes.value().len() <= MAX_RECORD,"oversized forge receipt");
            let value: Receipt = serde_json::from_slice(bytes.value())?; oid(&value.request_sha256)?;
            receipt(bytes.value(),&value.request_sha256)?;
            row(out,&mut hash,"receipt",id.value().as_bytes(),bytes.value())?; counts[2] += 1;
        }
        let digest: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        emit(out,&json!({"kind":"end","threads":counts[0],"events":counts[1],"receipts":counts[2],"sha256":digest}))
    }
}

pub fn run(command: Command) -> Result<()> {
    let repo = crate::git::discover()?;
    let db = ActionDb::open(&repo,true)?;
    let request = match command {
        Command::Control => serde_json::from_slice(&bounded_read(io::stdin().lock(),MAX_REQUEST)?)?,
        Command::Show { number } => Request::Get { number },
        Command::List { after,limit } => Request::List { after,limit },
        Command::Timeline { number,after,limit } => Request::Timeline { number,after,limit },
        Command::Check { number } => Request::Readiness { number },
        Command::Export => return db.forge_export(&mut io::stdout().lock()),
    };
    print_json(&db.forge_control(&repo,request)?)
}

#[cfg(test)]
#[path = "actions_forge_tests.rs"]
mod tests;
