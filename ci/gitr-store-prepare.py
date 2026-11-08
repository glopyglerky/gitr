#!/usr/bin/env python3
"""Stage explicit remote-action integration; CI never updates branch refs."""
from pathlib import Path

path = Path('rust/gitr/src/actions.rs')
text = path.read_text()
if 'mod remote_store;' not in text:
    text += '\n#[path = "actions_remote.rs"]\nmod remote_store;\n'
    needle = 'pub enum Command {'
    assert text.count(needle) == 1
    text = text.replace(needle, needle + '''
    /// Explicit opt-in to an independent local SurrealDB storage service.
    Remote {
        #[arg(long)]
        endpoint: String,
        #[arg(long)]
        repository: String,
        #[command(subcommand)]
        command: remote_store::Command,
    },''')
    needle = 'pub fn run(command: Command) -> Result<()> {\n    let repo = crate::git::discover()?;'
    assert text.count(needle) == 1
    text = text.replace(needle, needle + '''
    if let Command::Remote { endpoint, repository, command } = command {
        return remote_store::run(&repo, &endpoint, &repository, command);
    }''')
    needle = 'Command::Control => unreachable!'
    assert text.count(needle) == 1
    text = text.replace(needle, 'Command::Remote { .. } | Command::Control => unreachable!')
    path.write_text(text)

path = Path('rust/gitr/src/actions_remote.rs')
text = path.read_text()
if 'use std::time::Duration;' not in text:
    text = text.replace('use super::*;', 'use super::*;\nuse std::time::Duration;')
path.write_text(text)

path = Path('rust/gitr-store/src/main.rs')
text = path.read_text().replace('Context, Result, bail, ensure', 'Context, Result, ensure')
text = text.replace('RecordId::from((table, id.to_owned()))', 'RecordId::new(table, id.to_owned())')
if 'deadline: Option<u64>' not in text:
    needle = 'async fn publish(&self, job: &Job, previous: Option<i64>, object: Option<(&str, &str)>)'
    assert needle in text
    text = text.replace(needle, 'async fn publish(&self, job: &Job, previous: Option<i64>, object: Option<(&str, &str)>, deadline: Option<u64>)')
    text = text.replace('WHERE revision = $expected RETURN AFTER', 'WHERE revision = $expected AND ($deadline = NONE OR time::millis() < $deadline) RETURN AFTER')
    text = text.replace('.bind(("expected", previous)).bind(("next", next))', '.bind(("expected", previous)).bind(("next", next)).bind(("deadline", deadline.map(i64::try_from).transpose()?))')
    text = text.replace('self.publish(&job, None, Some((&spec.input_sha256, input_hex))).await?', 'self.publish(&job, None, Some((&spec.input_sha256, input_hex)), None).await?')
    text = text.replace('self.publish(&job, Some(row.revision), None).await?', 'self.publish(&job, Some(row.revision), None, None).await?')
    text = text.replace('job.status = if *exit_code == Some(0)', 'let deadline = job.lease_until_ms;\n                job.status = if *exit_code == Some(0)')
    text = text.replace('output_oid.as_deref().map(|id| (id, output_hex.as_str()))).await?', 'output_oid.as_deref().map(|id| (id, output_hex.as_str())), deadline).await?')
if 'PermissionsExt' not in text:
    needle = 'if !root.exists() { fs::create_dir_all(root)?; }'
    assert needle in text
    text = text.replace(needle, '''let new_root = !root.exists();
    if new_root { fs::create_dir_all(root)?; }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Only this OS account may inspect the local authority's durable state.
        if new_root { fs::set_permissions(root, fs::Permissions::from_mode(0o700))?; }
        ensure!(fs::symlink_metadata(root)?.permissions().mode() & 0o077 == 0,
            "storage directory must not be accessible to group or other users");
    }''')
if 'mod transactions;' not in text:
    start = text.index('fn retryable(error: &anyhow::Error) -> bool {')
    end = text.index('\nimpl Request {', start)
    text = text[:start] + '''fn retryable(error: &anyhow::Error) -> bool {
    error.downcast_ref::<surrealdb::Error>().is_some_and(transactions::retryable)
}
''' + text[end:]
    text = text.replace('types::{QueryError, RecordId, SurrealValue}', 'types::{RecordId, SurrealValue}')
    text = text.replace('.await?.check()?', '.await.and_then(transactions::check)?')
    text += '\nmod transactions;\n'
path.write_text(text)
