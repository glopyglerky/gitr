//! Rebuildable repository catalog. Git object IDs remain authoritative.
//!
//! A completed snapshot is an atomic set of tree entries for one immutable commit.
//! Its vectors are deterministic hashed ASCII token frequencies, not learned
//! semantic embeddings. Graph edges describe Git tree membership, not calls or
//! symbol resolution. Several local processes can open the catalog; redb still
//! serializes write transactions. Opening a new writable handle can also wait
//! for an active writer; read-only handles can read the last committed snapshot.
//! Distributed replication is not implemented.
//!
//! Traversal queues live in the transaction, not a repository-sized in-memory
//! collection. Application buffers hold one bounded metadata/source object and
//! search retains at most `limit` hits. Gix pack/delta decoding has its own buffers;
//! `max_blob_bytes` is an input-size bound, not an operating-system RAM limit.

use std::{
    cmp::Ordering,
    collections::BTreeSet,
    fs,
    io::{self, Write},
    path::PathBuf,
};

use anyhow::{Context, Result, bail, ensure};
use clap::Subcommand;
use gix::bstr::ByteSlice;
use redb::{ConcurrencyMode, Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

pub const VECTOR_MODEL: &str = "gitr-ascii-fnv1a-tf-l2-exact-v2";
pub const VECTOR_DIMENSIONS: usize = 512;
pub const DEFAULT_MAX_BLOB_BYTES: u64 = 1024 * 1024;
pub const MAX_BLOB_BYTES: u64 = 8 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 64 * 1024;
const CACHE_BYTES: usize = 16 * 1024 * 1024;
const SCHEMA_VERSION: u64 = 2;
const SNAPSHOTS: TableDefinition<&str, &[u8]> = TableDefinition::new("snapshots.v1");
const MEMBERS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("members.v1");
const FEATURES: TableDefinition<&str, &[u8]> = TableDefinition::new("features.v1");
const META: TableDefinition<&str, u64> = TableDefinition::new("meta.v1");
const QUEUE: TableDefinition<u64, &[u8]> = TableDefinition::new("pending.v1");

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Atomically index a committed snapshot; worktree changes are not included.
    Index {
        #[arg(default_value = "HEAD")]
        revision: String,
        /// Maximum source blob size to tokenize, at most 8 MiB.
        #[arg(long, default_value_t = DEFAULT_MAX_BLOB_BYTES)]
        max_blob_bytes: u64,
    },
    /// Rank by exact query-term overlap, then lexical cosine; no network calls.
    Search {
        query: String,
        #[arg(long, default_value = "HEAD")]
        revision: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Stream factual commit/tree/path/object relations as JSON Lines.
    Graph {
        #[arg(long, default_value = "HEAD")]
        revision: String,
        /// Exact Git path relative to the repository root.
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

/// Paths in Git are bytes. `hex` is lossless even when `utf8` is absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawPath {
    pub hex: String,
    pub utf8: Option<String>,
}

impl RawPath {
    fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            hex: encode_hex(bytes),
            utf8: std::str::from_utf8(bytes).ok().map(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexReport {
    pub commit_oid: String,
    pub root_tree_oid: String,
    pub object_format: String,
    pub schema_version: u64,
    pub generation: u64,
    pub vector_model: String,
    pub vector_dimensions: usize,
    pub learned_semantic_embeddings: bool,
    pub max_blob_bytes: u64,
    pub files: u64,
    pub directories: u64,
    pub submodules: u64,
    pub indexed_files: u64,
    pub skipped_files: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub path: RawPath,
    pub blob_oid: String,
    pub bytes: u64,
    pub matched_query_terms: usize,
    pub lexical_cosine: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchReport {
    pub commit_oid: String,
    pub vector_model: String,
    pub learned_semantic_embeddings: bool,
    pub indexed_files: u64,
    pub skipped_files: u64,
    pub query_term_count: usize,
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Member {
    path: RawPath,
    parent_tree_oid: String,
    object_oid: String,
    object_kind: String,
    mode: u32,
    bytes: Option<u64>,
    lexical_state: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Feature {
    model: String,
    dimensions: usize,
    state: String,
    vector: Vec<f32>,
    /// Sorted, distinct lowercase ASCII tokens prevent hash-only matches.
    terms: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Pending {
    path: Vec<u8>,
    oid: String,
    parent_tree_oid: String,
    mode: u32,
    depth: u32,
}

struct Resolved {
    commit: gix::ObjectId,
    tree: gix::ObjectId,
}

pub fn catalog_path(repo: &gix::Repository) -> PathBuf {
    repo.common_dir().join("gitr").join("catalog.redb")
}

pub fn run(command: Command) -> Result<()> {
    let repo = crate::git::discover()?;
    let mut stdout = io::stdout().lock();
    match command {
        Command::Index {
            revision,
            max_blob_bytes,
        } => json_line(&mut stdout, &index(&repo, &revision, max_blob_bytes)?),
        Command::Search {
            query,
            revision,
            limit,
        } => json_line(&mut stdout, &search(&repo, &query, &revision, limit)?),
        Command::Graph { revision, path } => {
            let bytes = path
                .as_ref()
                .map(|p| gix::path::os_str_into_bstr(p.as_os_str()))
                .transpose()?;
            write_graph(
                &repo,
                &revision,
                bytes.as_ref().map(|p| p.as_bytes()),
                &mut stdout,
            )
        }
    }
}

fn builder() -> redb::Builder {
    let mut builder = Database::builder();
    builder
        .set_concurrency_mode(ConcurrencyMode::MultiWriter)
        .set_cache_size(CACHE_BYTES);
    builder
}

fn checked_catalog_path(repo: &gix::Repository, create: bool) -> Result<PathBuf> {
    let path = catalog_path(repo);
    let directory = path.parent().expect("catalog path has a parent");
    if create {
        fs::create_dir_all(directory).context("create repository catalog directory")?;
    }
    for candidate in [directory, path.as_path()] {
        match fs::symlink_metadata(candidate) {
            Ok(metadata) => {
                ensure!(
                    !metadata.file_type().is_symlink(),
                    "catalog paths must not be symlinks: {}",
                    candidate.display()
                );
                if candidate == directory {
                    ensure!(metadata.is_dir(), "catalog parent must be a directory");
                } else {
                    ensure!(metadata.is_file(), "catalog must be a regular file");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && create => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                bail!("no catalog; run `gitr db index` for the requested revision first")
            }
            Err(error) => return Err(error).context("inspect repository catalog path"),
        }
    }
    Ok(path)
}

/// Resolve once, then use immutable IDs even if the named branch moves.
fn resolve(repo: &gix::Repository, revision: &str) -> Result<Resolved> {
    ensure!(
        !revision.is_empty() && revision.len() <= 4096,
        "revision must contain 1..4096 bytes"
    );
    let mut id = repo
        .rev_parse_single(revision.as_bytes().as_bstr())?
        .detach();
    for _ in 0..32 {
        let header = repo
            .find_header(id)
            .context("read revision object header")?;
        ensure!(
            header.size() <= MAX_METADATA_BYTES,
            "revision metadata exceeds the 8 MiB indexing limit"
        );
        ensure!(
            matches!(
                header.kind(),
                gix::objs::Kind::Commit | gix::objs::Kind::Tag
            ),
            "revision must identify a commit or an annotated tag to a commit"
        );
        let object = repo.find_object(id)?;
        verify_object(repo, id, object.kind, &object.data)?;
        match object.kind {
            gix::objs::Kind::Commit => {
                let commit = object.into_commit();
                return Ok(Resolved {
                    commit: id,
                    tree: commit.tree_id()?.detach(),
                });
            }
            gix::objs::Kind::Tag => id = object.into_tag().target_id()?.detach(),
            _ => bail!("revision object kind changed during resolution"),
        }
    }
    bail!("annotated tag chain exceeds the 32-object indexing limit")
}

fn verify_object(
    repo: &gix::Repository,
    expected: gix::ObjectId,
    kind: gix::objs::Kind,
    data: &[u8],
) -> Result<()> {
    let actual = gix::objs::compute_hash(repo.object_hash(), kind, data)?;
    ensure!(
        actual == expected,
        "Git object content does not match its object ID: {expected}"
    );
    Ok(())
}

/// Index one immutable commit atomically. A failed traversal publishes no partial snapshot.
/// The writer lock is held for this scan; readers keep their previous MVCC snapshot.
pub fn index(repo: &gix::Repository, revision: &str, max_blob_bytes: u64) -> Result<IndexReport> {
    ensure!(
        max_blob_bytes <= MAX_BLOB_BYTES,
        "--max-blob-bytes must not exceed {MAX_BLOB_BYTES}"
    );
    let resolved = resolve(repo, revision)?;
    let commit_oid = resolved.commit.to_string();
    let database = builder()
        .create(checked_catalog_path(repo, true)?)
        .context("open redb 4.5 MultiWriter catalog (requires supported local file locking)")?;
    let mut transaction = database.begin_write()?;
    transaction.set_durability(redb::Durability::Immediate)?;
    let report;
    {
        let mut meta = transaction.open_table(META)?;
        let schema = meta.get("schema")?.map(|value| value.value());
        ensure!(
            schema.is_none_or(|version| version == SCHEMA_VERSION),
            "unsupported catalog schema; rebuild the derived catalog with a matching gitr version"
        );
        meta.insert("schema", SCHEMA_VERSION)?;
        let mut snapshots = transaction.open_table(SNAPSHOTS)?;
        let previous: Option<IndexReport> = snapshots
            .get(commit_oid.as_str())?
            .map(|value| serde_json::from_slice(value.value()))
            .transpose()?;
        if let Some(previous) = previous.as_ref() {
            validate_snapshot(previous, &commit_oid)?;
            if previous.max_blob_bytes == max_blob_bytes {
                return Ok(previous.clone());
            }
        }
        let generation = meta
            .get("generation")?
            .map(|value| value.value())
            .unwrap_or(0)
            .checked_add(1)
            .context("catalog generation exhausted")?;
        ensure!(generation < u64::MAX, "catalog generation exhausted");
        meta.insert("generation", generation)?;
        let mut members = transaction.open_table(MEMBERS)?;
        let mut features = transaction.open_table(FEATURES)?;
        let mut queue = transaction.open_table(QUEUE)?;
        ensure!(
            queue.first()?.is_none(),
            "catalog contains an unexpected pending traversal; rebuild the derived catalog"
        );
        let mut next_queue = 1u64;
        let root = Pending {
            path: Vec::new(),
            oid: resolved.tree.to_string(),
            parent_tree_oid: String::new(),
            mode: 0o040000,
            depth: 0,
        };
        queue.insert(0, serde_json::to_vec(&root)?.as_slice())?;
        let mut result = IndexReport {
            commit_oid: commit_oid.clone(),
            root_tree_oid: resolved.tree.to_string(),
            object_format: repo.object_hash().to_string(),
            schema_version: SCHEMA_VERSION,
            generation,
            vector_model: VECTOR_MODEL.into(),
            vector_dimensions: VECTOR_DIMENSIONS,
            learned_semantic_embeddings: false,
            max_blob_bytes,
            files: 0,
            directories: 0,
            submodules: 0,
            indexed_files: 0,
            skipped_files: 0,
        };
        loop {
            let next = {
                match queue.first()? {
                    Some((key, value)) => Some((
                        key.value(),
                        serde_json::from_slice::<Pending>(value.value())?,
                    )),
                    None => None,
                }
            };
            let Some((queue_key, pending)) = next else {
                break;
            };
            queue.remove(queue_key)?;
            let oid = gix::ObjectId::from_hex(pending.oid.as_bytes())?;
            let mut member = Member {
                path: RawPath::from_bytes(&pending.path),
                parent_tree_oid: pending.parent_tree_oid,
                object_oid: pending.oid.clone(),
                object_kind: String::new(),
                mode: pending.mode,
                bytes: None,
                lexical_state: "not-applicable".into(),
            };
            match pending.mode {
                0o040000 => {
                    result.directories += 1;
                    member.object_kind = "tree".into();
                    ensure!(
                        pending.depth <= 1024,
                        "tree nesting exceeds 1024 directories"
                    );
                    let header = repo
                        .find_header(oid)
                        .context("missing tree object; fetch required objects before indexing")?;
                    ensure!(
                        header.kind() == gix::objs::Kind::Tree
                            && header.size() <= MAX_METADATA_BYTES,
                        "tree {oid} has an invalid kind or exceeds the 8 MiB indexing limit"
                    );
                    let tree = repo.find_tree(oid)?;
                    verify_object(repo, oid, gix::objs::Kind::Tree, &tree.data)?;
                    member.bytes = Some(tree.data.len() as u64);
                    for entry in gix::objs::TreeRefIter::from_bytes(&tree.data, repo.object_hash())
                    {
                        let entry = entry?;
                        let filename = entry.filename.as_bytes();
                        ensure!(
                            !filename.is_empty()
                                && filename != b"."
                                && filename != b".."
                                && !filename.contains(&b'/')
                                && !filename.contains(&0),
                            "invalid Git tree entry name"
                        );
                        let mut path = pending.path.clone();
                        if !path.is_empty() {
                            path.push(b'/');
                        }
                        path.extend_from_slice(filename);
                        ensure!(
                            path.len() <= MAX_PATH_BYTES,
                            "Git path exceeds the 64 KiB indexing limit"
                        );
                        let child = Pending {
                            path,
                            oid: entry.oid.to_string(),
                            parent_tree_oid: pending.oid.clone(),
                            mode: u32::from(entry.mode.value()),
                            depth: pending.depth + 1,
                        };
                        queue.insert(next_queue, serde_json::to_vec(&child)?.as_slice())?;
                        next_queue = next_queue.checked_add(1).context("too many tree entries")?;
                    }
                }
                0o160000 => {
                    result.submodules += 1;
                    member.object_kind = "submodule-commit".into();
                    member.lexical_state = "submodule-not-indexed".into();
                }
                0o100644 | 0o100755 | 0o120000 => {
                    result.files += 1;
                    member.object_kind = "blob".into();
                    let header = repo
                        .find_header(oid)
                        .context("missing blob object; fetch required objects before indexing")?;
                    ensure!(
                        header.kind() == gix::objs::Kind::Blob,
                        "tree entry {oid} does not reference a blob"
                    );
                    member.bytes = Some(header.size());
                    if pending.mode == 0o120000 {
                        member.lexical_state = "symlink-not-followed".into();
                    } else if header.size() > max_blob_bytes {
                        member.lexical_state = "too-large".into();
                    } else {
                        let key = format!("{VECTOR_MODEL}:{oid}");
                        let cached: Option<Feature> = features
                            .get(key.as_str())?
                            .map(|value| serde_json::from_slice(value.value()))
                            .transpose()?;
                        let feature = if let Some(feature) = cached {
                            feature
                        } else {
                            let blob = repo.find_blob(oid)?;
                            ensure!(
                                blob.data.len() as u64 == header.size()
                                    && blob.data.len() as u64 <= max_blob_bytes,
                                "blob size changed or exceeded its declared bound"
                            );
                            verify_object(repo, oid, gix::objs::Kind::Blob, &blob.data)?;
                            let feature = extract(&blob.data)?;
                            features
                                .insert(key.as_str(), serde_json::to_vec(&feature)?.as_slice())?;
                            feature
                        };
                        validate_feature(&feature)?;
                        member.lexical_state = feature.state;
                    }
                    if member.lexical_state == "indexed" {
                        result.indexed_files += 1;
                    } else {
                        result.skipped_files += 1;
                    }
                }
                _ => bail!("unsupported Git tree entry mode {:o}", pending.mode),
            }
            let key = member_key(generation, &pending.path);
            ensure!(
                members
                    .insert(key.as_slice(), serde_json::to_vec(&member)?.as_slice())?
                    .is_none(),
                "duplicate path in Git tree"
            );
        }
        if let Some(previous) = previous {
            // Retire the replaced generation only after the replacement completed.
            // MVCC readers that already opened the old generation remain consistent.
            remove_generation(&mut members, previous.generation)?;
        }
        snapshots.insert(commit_oid.as_str(), serde_json::to_vec(&result)?.as_slice())?;
        report = result;
    }
    transaction
        .commit()
        .context("commit complete repository snapshot")?;
    Ok(report)
}

fn remove_generation(members: &mut redb::Table<'_, &[u8], &[u8]>, generation: u64) -> Result<()> {
    let start = generation.to_be_bytes();
    let end = (generation + 1).to_be_bytes();
    loop {
        let key = members
            .range(start.as_slice()..end.as_slice())?
            .next()
            .transpose()?
            .map(|(key, _)| key.value().to_vec());
        match key {
            Some(key) => {
                members.remove(key.as_slice())?;
            }
            None => break,
        }
    }
    Ok(())
}

fn read_snapshot(transaction: &redb::ReadTransaction, commit_oid: &str) -> Result<IndexReport> {
    let snapshots = transaction
        .open_table(SNAPSHOTS)
        .context("catalog has no completed snapshots; run `gitr db index`")?;
    let bytes = snapshots.get(commit_oid)?.with_context(|| {
        format!("commit {commit_oid} is not indexed; run `gitr db index {commit_oid}`")
    })?;
    let snapshot = serde_json::from_slice(bytes.value())?;
    validate_snapshot(&snapshot, commit_oid)?;
    Ok(snapshot)
}

fn validate_snapshot(snapshot: &IndexReport, commit_oid: &str) -> Result<()> {
    ensure!(
        snapshot.commit_oid == commit_oid
            && snapshot.schema_version == SCHEMA_VERSION
            && snapshot.vector_model == VECTOR_MODEL
            && snapshot.vector_dimensions == VECTOR_DIMENSIONS
            && !snapshot.learned_semantic_embeddings,
        "catalog snapshot schema or model does not match this gitr build; rebuild the derived catalog"
    );
    ensure!(
        snapshot.generation > 0 && snapshot.generation < u64::MAX,
        "invalid catalog snapshot generation"
    );
    Ok(())
}

fn validate_feature(feature: &Feature) -> Result<()> {
    ensure!(
        feature.model == VECTOR_MODEL && feature.dimensions == VECTOR_DIMENSIONS,
        "incompatible lexical feature model"
    );
    if feature.state == "indexed" {
        ensure!(
            feature.vector.len() == VECTOR_DIMENSIONS
                && feature
                    .vector
                    .iter()
                    .all(|x| x.is_finite() && *x >= 0.0 && *x <= 1.0),
            "invalid stored lexical vector"
        );
        ensure!(
            !feature.terms.is_empty()
                && feature.terms.windows(2).all(|pair| pair[0] < pair[1])
                && feature.terms.iter().all(|term| {
                    !term.is_empty()
                        && term
                            .bytes()
                            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                })
                && feature
                    .terms
                    .iter()
                    .try_fold(0u64, |total, term| total.checked_add(term.len() as u64))
                    .is_some_and(|bytes| bytes <= MAX_BLOB_BYTES),
            "invalid stored exact lexical terms"
        );
    } else {
        ensure!(
            feature.vector.is_empty() && feature.terms.is_empty(),
            "unexpected lexical features for skipped content"
        );
    }
    Ok(())
}

/// Search only the requested commit's completed snapshot. No stale fallback.
pub fn search(
    repo: &gix::Repository,
    query: &str,
    revision: &str,
    limit: usize,
) -> Result<SearchReport> {
    ensure!(
        (1..=1000).contains(&limit),
        "--limit must be between 1 and 1000"
    );
    ensure!(query.len() <= 16 * 1024, "query exceeds 16 KiB");
    let query_terms = unique_terms(query.as_bytes());
    let query_vector = lexical_vector(query.as_bytes());
    ensure!(
        !query_terms.is_empty(),
        "query must contain ASCII letters or digits"
    );
    let resolved = resolve(repo, revision)?;
    let commit_oid = resolved.commit.to_string();
    let database = builder().open_read_only(checked_catalog_path(repo, false)?)?;
    let transaction = database.begin_read()?;
    let snapshot = read_snapshot(&transaction, &commit_oid)?;
    let members = transaction.open_table(MEMBERS)?;
    let features = transaction.open_table(FEATURES)?;
    let start = snapshot.generation.to_be_bytes();
    let end = (snapshot.generation + 1).to_be_bytes();
    let mut hits = Vec::with_capacity(limit);
    for entry in members.range(start.as_slice()..end.as_slice())? {
        let (_, value) = entry?;
        let member: Member = serde_json::from_slice(value.value())?;
        if member.lexical_state != "indexed" {
            continue;
        }
        let key = format!("{VECTOR_MODEL}:{}", member.object_oid);
        let bytes = features
            .get(key.as_str())?
            .context("completed snapshot references a missing lexical vector")?;
        let feature: Feature = serde_json::from_slice(bytes.value())?;
        validate_feature(&feature)?;
        ensure!(
            feature.state == "indexed",
            "completed snapshot references a skipped lexical vector"
        );
        let matched_query_terms = query_terms
            .iter()
            .filter(|term| feature.terms.binary_search(term).is_ok())
            .count();
        if matched_query_terms == 0 {
            continue;
        }
        let score: f64 = query_vector
            .iter()
            .zip(&feature.vector)
            .map(|(a, b)| f64::from(*a) * f64::from(*b))
            .sum();
        if score <= 0.0 {
            continue;
        }
        let candidate = SearchHit {
            path: member.path,
            blob_oid: member.object_oid,
            bytes: member.bytes.context("indexed blob lacks size")?,
            matched_query_terms,
            lexical_cosine: score.min(1.0),
        };
        let position = hits
            .binary_search_by(|existing| compare_hits(existing, &candidate))
            .unwrap_or_else(|position| position);
        if position < limit {
            hits.insert(position, candidate);
            if hits.len() > limit {
                hits.pop();
            }
        }
    }
    Ok(SearchReport {
        commit_oid,
        vector_model: VECTOR_MODEL.into(),
        learned_semantic_embeddings: false,
        indexed_files: snapshot.indexed_files,
        skipped_files: snapshot.skipped_files,
        query_term_count: query_terms.len(),
        hits,
    })
}

fn compare_hits(a: &SearchHit, b: &SearchHit) -> Ordering {
    b.matched_query_terms
        .cmp(&a.matched_query_terms)
        .then_with(|| b.lexical_cosine.total_cmp(&a.lexical_cosine))
        .then_with(|| a.path.hex.cmp(&b.path.hex))
}

/// Stream JSON Lines without materializing the graph in memory. A path filter is
/// an exact byte path, not a glob, filesystem traversal, or text substitution.
pub fn write_graph(
    repo: &gix::Repository,
    revision: &str,
    path: Option<&[u8]>,
    output: &mut impl Write,
) -> Result<()> {
    let resolved = resolve(repo, revision)?;
    let commit_oid = resolved.commit.to_string();
    let database = builder().open_read_only(checked_catalog_path(repo, false)?)?;
    let transaction = database.begin_read()?;
    let snapshot = read_snapshot(&transaction, &commit_oid)?;
    let members = transaction.open_table(MEMBERS)?;
    let selected = if let Some(path) = path {
        let key = member_key(snapshot.generation, path);
        Some(
            members
                .get(key.as_slice())?
                .context("path does not exist in the indexed commit")?
                .value()
                .to_vec(),
        )
    } else {
        None
    };
    json_line(
        output,
        &serde_json::json!({"relation":"commit_tree","commit_oid":commit_oid,"tree_oid":snapshot.root_tree_oid,"snapshot":snapshot,"graph_kind":"git-tree-membership","call_graph":false}),
    )?;
    if let Some(selected) = selected {
        let member: Member = serde_json::from_slice(&selected)?;
        if !member.path.hex.is_empty() {
            json_line(
                output,
                &serde_json::json!({"relation":"tree_entry","entry":member}),
            )?;
        }
    } else {
        let start = snapshot.generation.to_be_bytes();
        let end = (snapshot.generation + 1).to_be_bytes();
        for entry in members.range(start.as_slice()..end.as_slice())? {
            let (_, value) = entry?;
            let member: Member = serde_json::from_slice(value.value())?;
            if !member.path.hex.is_empty() {
                json_line(
                    output,
                    &serde_json::json!({"relation":"tree_entry","entry":member}),
                )?;
            }
        }
    }
    Ok(())
}

fn extract(bytes: &[u8]) -> Result<Feature> {
    let (state, vector) = if bytes.contains(&0) {
        ("binary", Vec::new())
    } else if std::str::from_utf8(bytes).is_err() {
        ("non-utf8", Vec::new())
    } else {
        // Use the filter's decoder so CRLF and legacy pointer spelling have
        // exactly the same coverage policy as their LF v1 equivalents.
        match crate::lfs::Pointer::decode(bytes) {
            Ok(Some(_)) if !bytes.is_empty() => ("lfs-pointer-not-asset-content", Vec::new()),
            Err(_) => ("unparsed-lfs-pointer", Vec::new()),
            _ => {
                let vector = lexical_vector(bytes);
                if vector.iter().all(|x| *x == 0.0) {
                    ("no-ascii-tokens", Vec::new())
                } else {
                    ("indexed", vector)
                }
            }
        }
    };
    Ok(Feature {
        model: VECTOR_MODEL.into(),
        dimensions: VECTOR_DIMENSIONS,
        state: state.into(),
        vector,
        terms: if state == "indexed" {
            unique_terms(bytes)
        } else {
            Vec::new()
        },
    })
}

fn ascii_tokens(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split(|byte| !byte.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
}

/// Retain only distinct terms from one bounded source blob, never a whole repo.
fn unique_terms(bytes: &[u8]) -> Vec<String> {
    ascii_tokens(bytes)
        .map(|token| {
            token
                .iter()
                .map(|byte| char::from(byte.to_ascii_lowercase()))
                .collect::<String>()
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Stable FNV-1a over case-folded ASCII alphanumeric tokens. Punctuation,
/// underscores and non-ASCII bytes delimit tokens. Counts use log TF then L2
/// normalization. Exact term overlap gates and precedes this collision-prone score.
fn lexical_vector(bytes: &[u8]) -> Vec<f32> {
    let mut counts = [0u32; VECTOR_DIMENSIONS];
    for token in ascii_tokens(bytes) {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in token {
            hash ^= u64::from(byte.to_ascii_lowercase());
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let count = &mut counts[(hash % VECTOR_DIMENSIONS as u64) as usize];
        *count = count.saturating_add(1);
    }
    let mut values: Vec<f32> = counts
        .into_iter()
        .map(|count| {
            if count == 0 {
                0.0
            } else {
                (1.0 + f64::from(count).ln()) as f32
            }
        })
        .collect();
    let norm = values
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        .sqrt();
    if norm != 0.0 {
        for value in &mut values {
            *value = (f64::from(*value) / norm) as f32;
        }
    }
    values
}

fn member_key(generation: u64, path: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(8 + path.len());
    key.extend_from_slice(&generation.to_be_bytes());
    key.extend_from_slice(path);
    key
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 15)]));
    }
    output
}

fn json_line(output: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    writeln!(output)?;
    Ok(())
}
