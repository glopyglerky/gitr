//! Read-only, snapshot-consistent durable action export for migration rehearsal.
//! This is not a cutover or importer. Writes may continue after this snapshot;
//! a live export must never be used as an authoritative migration checkpoint.
use super::*;
use serde_json::json;

/// Deterministic hexadecimal with no dependency on platform debug formatting.
fn encode_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("write into String");
    }
    out
}
fn line(out: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *out, value)?;
    out.write_all(b"\n")?;
    Ok(())
}
fn mix_record(hasher: &mut Sha256, kind: &[u8], key: &[u8], value: &[u8]) {
    // Length framing makes the digest unambiguous between distinct records.
    hasher.update(kind);
    hasher.update((key.len() as u64).to_be_bytes());
    hasher.update(key);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

impl ActionDb {
    /// Stream one consistent redb read snapshot as newline-delimited JSON.
    /// Every immutable payload is rehashed. Every receipt references objects
    /// present in the same snapshot. Active running jobs prevent an export.
    /// The terminal checksum is over original stored records, not pretty JSON.
    pub fn export(&self, out: &mut impl Write) -> Result<()> {
        let tx = self.db.begin_read()?;
        ensure!(
            tx.open_table(META)?.get("schema")?.map(|v| v.value()) == Some(1),
            "unsupported action schema"
        );
        let objects = tx.open_table(OBJECTS)?;
        let jobs = tx.open_table(JOBS)?;
        let mut hash = Sha256::new();
        let mut object_count = 0u64;
        let mut job_count = 0u64;

        // Fail closed before writing any bytes when the selected snapshot has an
        // active worker. A quiescent snapshot alone still is not a cutover fence.
        for row in jobs.iter()? {
            let (id, value) = row?;
            key(id.value())?;
            ensure!(value.value().len() <= 64 * 1024, "oversized action receipt");
            let job: Job = serde_json::from_slice(value.value())?;
            ensure!(job.id == id.value(), "action job identifier does not match its key");
            ensure!(job.spec.profile == PROFILE, "unknown action execution profile");
            ensure!(job.status != Status::Running, "cannot export a snapshot with running workers");
            ensure!(
                (job.status == Status::Passed) == job.artifact_sha256.is_some(),
                "action receipt has inconsistent successful artifact state"
            );
            for reference in [
                Some(job.spec.module_sha256.as_str()),
                Some(job.spec.input_sha256.as_str()),
                job.artifact_sha256.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                oid(reference)?;
                let bytes = objects.get(reference)?.context("receipt references missing action object")?;
                ensure!(
                    bytes.value().len() <= MAX_BYTES && digest(bytes.value()) == reference,
                    "receipt references corrupt action object"
                );
            }
        }
        line(out, &json!({"kind":"header","format":"gitr-action-export-v1","schema":1}))?;

        for row in objects.iter()? {
            let (id, value) = row?;
            let id = id.value();
            let bytes = value.value();
            oid(id)?;
            ensure!(bytes.len() <= MAX_BYTES && digest(bytes) == id, "action object integrity failure");
            mix_record(&mut hash, b"object\0", id.as_bytes(), bytes);
            line(out, &json!({"kind":"object","sha256":id,"data_hex":encode_hex(bytes)}))?;
            object_count = object_count.checked_add(1).context("export object counter overflow")?;
        }
        for row in jobs.iter()? {
            let (id, value) = row?;
            let id = id.value();
            let bytes = value.value();
            let job: Job = serde_json::from_slice(bytes)?;
            mix_record(&mut hash, b"job\0", id.as_bytes(), bytes);
            line(out, &json!({"kind":"job","id":id,"json_hex":encode_hex(bytes),"record":job}))?;
            job_count = job_count.checked_add(1).context("export job counter overflow")?;
        }
        line(out, &json!({
            "kind":"end", "objects":object_count, "jobs":job_count,
            "sha256":encode_hex(&hash.finalize())
        }))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (tempfile::TempDir, ActionDb, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = ActionDb::open_path(&dir.path().join("state.redb"), true).unwrap();
        let wasm = wat::parse_str(
            r#"(module (memory (export "memory") 1) (func (export "run") (result i32) i32.const 0))"#,
        )
        .unwrap();
        let module = db.install(&wasm).unwrap();
        (dir, db, module)
    }

    fn input(contents: &[u8]) -> SnapshotInput {
        SnapshotInput {
            commit: "1".repeat(40),
            blob: "2".repeat(40),
            path: "src/main.rs".to_owned(),
            bytes: contents.to_vec(),
        }
    }

    #[test]
    fn deterministic_snapshot_export_survives_reopen_and_does_not_modify_jobs() {
        let (dir, db, module) = state();
        db.submit("policy", &module, input(b"immutable input")).unwrap();
        let before = db.get("policy").unwrap();
        let mut first = Vec::new();
        db.export(&mut first).unwrap();
        assert_eq!(db.get("policy").unwrap(), before);
        let lines: Vec<serde_json::Value> = first
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(lines.first().unwrap()["format"], "gitr-action-export-v1");
        assert_eq!(lines.last().unwrap()["kind"], "end");
        assert_eq!(lines.last().unwrap()["objects"], 2);
        assert_eq!(lines.last().unwrap()["jobs"], 1);
        assert!(lines.iter().any(|v| v["kind"] == "job" && v["id"] == "policy"));
        drop(db);
        let db = ActionDb::open_path(&dir.path().join("state.redb"), false).unwrap();
        let mut second = Vec::new();
        db.export(&mut second).unwrap();
        assert_eq!(second, first);
    }

    #[test]
    fn export_refuses_active_workers_and_missing_or_corrupt_references() {
        let (_dir, db, module) = state();
        db.submit("policy", &module, input(b"contents")).unwrap();
        let claim = db.claim("policy", || Ok(123)).unwrap().unwrap();
        assert!(db.export(&mut Vec::new()).is_err());
        db.cancel(&claim.id).unwrap();
        let mut out = Vec::new();
        db.export(&mut out).unwrap();
        let tx = db.db.begin_write().unwrap();
        tx.open_table(OBJECTS)
            .unwrap()
            .insert(module.as_str(), b"corrupt".as_slice())
            .unwrap();
        tx.commit().unwrap();
        assert!(db.export(&mut Vec::new()).is_err());
    }
}
