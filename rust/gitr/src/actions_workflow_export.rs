//! Workflow ownership and plans are part of the same export snapshot as jobs.
//! A v1 export must never silently omit the dependencies of managed actions.
use super::*;
use crate::actions::export as outfmt;
use serde_json::json;

pub(crate) fn verify_export_job(tx: &redb::ReadTransaction, job: &Job) -> Result<()> {
    if job.spec.profile == PROFILE { return Ok(()); }
    ensure!(job.spec.profile == MANAGED_PROFILE, "unknown action profile");
    ensure!(tx.open_table(META)?.get("workflow_schema")?.map(|v| v.value()) == Some(1), "workflow metadata is missing or unsupported");
    let owners = tx.open_table(OWNERS)?;
    let owner = owners.get(job.id.as_str())?.context("managed action has no exported owner")?;
    let runs = tx.open_table(RUNS)?;
    let value = runs.get(owner.value())?.context("managed action has no exported workflow")?;
    let run = decode(value.value(), owner.value())?;
    let task = run.plan.tasks.iter().find(|t| t.action_id == job.id).context("managed action is not in the exported plan")?;
    check_job(task, job)
}

pub(crate) fn export_preflight(tx: &redb::ReadTransaction) -> Result<bool> {
    let version = tx.open_table(META)?.get("workflow_schema")?.map(|v| v.value());
    let Some(version) = version else { return Ok(false); };
    ensure!(version == 1, "unsupported workflow export schema");
    let runs = tx.open_table(RUNS)?;
    let owners = tx.open_table(OWNERS)?;
    let jobs = tx.open_table(JOBS)?;
    let objects = tx.open_table(OBJECTS)?;
    let mut expected = 0u64;
    for row in runs.iter()? {
        let (id, bytes) = row?;
        let run = decode(bytes.value(), id.value())?;
        let source = objects.get(run.plan.workflow_sha256.as_str())?.context("workflow source missing")?;
        ensure!(source.value().len() <= 256 * 1024 && digest(source.value()) == run.plan.workflow_sha256, "workflow source corrupt");
        for task in &run.plan.tasks {
            ensure!(owners.get(task.action_id.as_str())?.map(|v| v.value() == id.value()).unwrap_or(false), "workflow ownership corrupt");
            let bytes = jobs.get(task.action_id.as_str())?.context("workflow action missing")?;
            check_job(task, &serde_json::from_slice(bytes.value())?)?;
            expected = expected.checked_add(1).context("workflow export counter overflow")?;
        }
    }
    let mut actual = 0u64;
    for row in owners.iter()? {
        let (id, owner) = row?;
        key(id.value())?;
        key(owner.value())?;
        let bytes = jobs.get(id.value())?.context("orphan workflow ownership")?;
        let job: Job = serde_json::from_slice(bytes.value())?;
        ensure!(job.spec.profile == MANAGED_PROFILE, "workflow owner points to an unmanaged action");
        verify_export_job(tx, &job)?;
        actual = actual.checked_add(1).context("workflow export counter overflow")?;
    }
    ensure!(actual == expected, "workflow ownership count mismatch");
    Ok(true)
}

pub(crate) fn export_records(tx: &redb::ReadTransaction, out: &mut impl Write, hash: &mut Sha256) -> Result<u64> {
    if tx.open_table(META)?.get("workflow_schema")?.is_none() { return Ok(0); }
    let mut count = 0u64;
    for row in tx.open_table(RUNS)?.iter()? {
        let (id, bytes) = row?;
        let run = decode(bytes.value(), id.value())?;
        outfmt::mix_record(hash, b"workflow\0", id.value().as_bytes(), bytes.value());
        outfmt::line(out, &json!({"kind":"workflow","id":id.value(),"json_hex":outfmt::encode_hex(bytes.value()),"record":run}))?;
        count = count.checked_add(1).context("workflow export counter overflow")?;
    }
    for row in tx.open_table(OWNERS)?.iter()? {
        let (id, owner) = row?;
        outfmt::mix_record(hash, b"workflow_owner\0", id.value().as_bytes(), owner.value().as_bytes());
        outfmt::line(out, &json!({"kind":"workflow_owner","id":id.value(),"run_id":owner.value()}))?;
    }
    Ok(count)
}
