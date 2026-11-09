//! The first checked receiver: bare SHA-1 repositories, bounded packs, no
//! receiving hooks, independent expected-old ref publication. Atomic batches
//! and hooks require separate qualification; the donor implementation is kept
//! for its conformance target, never used as a product fallback.
use super::{Args, DiagSink, RefOutcome};
use anyhow::{bail, Context, Result};
use grit_lib::config::ConfigSet;
use grit_lib::objects::{ObjectId, ObjectKind};
use grit_lib::odb::Odb;
use grit_lib::refs::{self, Ref};
use grit_lib::repo::Repository;
use std::collections::HashSet;
use std::io::{self, Read};

const MAX_COMMANDS: usize = 1024;
const MAX_PACK: u64 = 64 * 1024 * 1024;
const MAX_OBJECT: u64 = 16 * 1024 * 1024;
const MAX_EXPANDED: u64 = 128 * 1024 * 1024;
const MAX_OBJECTS: u32 = 10_000;

pub(crate) fn ensure_supported(repo: &Repository, config: &ConfigSet) -> Result<()> {
    if !repo.is_bare() { bail!("checked receive-pack currently requires a bare repository"); }
    if repo.odb.hash_algo() != grit_lib::objects::HashAlgo::Sha1 {
        bail!("checked receive-pack currently supports SHA-1 repositories only");
    }
    if repo.git_dir.join("shallow").exists() {
        bail!("checked receive-pack does not yet support shallow repositories");
    }
    if std::env::var_os("GIT_NAMESPACE").is_some_and(|name| !name.is_empty())
        || repo.git_dir.join("info/grafts").exists()
        || !refs::list_refs(&repo.git_dir, "refs/replace/")?.is_empty()
    {
        bail!("checked receive-pack does not yet support namespaces, grafts or replacement refs");
    }
    for event in ["pre-receive", "update", "post-receive", "post-update", "reference-transaction", "push-to-checkout", "proc-receive"] {
        let hooks = grit_lib::hooks::list_hooks_display_lines(Some(repo), event, config)
            .map_err(anyhow::Error::msg)?;
        if !hooks.is_empty() {
            bail!("checked receive-pack refuses configured {event} hooks until quarantine hook semantics are qualified");
        }
    }
    for key in ["core.alternateRefsCommand", "receive.procReceiveRefs", "receive.certNonceSeed", "receive.denyDeleteCurrent", "receive.fsckObjects", "receive.shallowUpdate"] {
        if config.get(key).is_some() { bail!("checked receive-pack does not yet support {key}"); }
    }
    if config.entries().iter().any(|entry| entry.key.starts_with("receive.fsck.")) {
        bail!("checked receive-pack does not yet support receive.fsck policies");
    }
    Ok(())
}

#[derive(Debug)]
struct Update { old: Option<ObjectId>, new: Option<ObjectId>, name: String }

fn parse_oid(text: &str) -> Result<Option<ObjectId>> {
    if text.len() != 40 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("receive-pack requires an exact 40-hex-digit SHA-1 object ID");
    }
    if text.bytes().all(|b| b == b'0') { Ok(None) } else { Ok(Some(ObjectId::from_hex(text)?)) }
}

// Parse raw pkt-lines: the shared presentation-oriented parser uses lossy UTF-8
// and permits truncated EOF, neither of which is acceptable for ref mutation.
fn read_commands(input: &mut impl Read) -> Result<(Vec<Update>, bool)> {
    let mut updates = Vec::new();
    let mut names = HashSet::new();
    let mut sideband = false;
    let mut total = 0usize;
    loop {
        let mut header = [0u8; 4];
        input.read_exact(&mut header).context("truncated receive-pack command header")?;
        let len = usize::from_str_radix(std::str::from_utf8(&header)?, 16)?;
        if len == 0 { break; }
        if !(5..=65520).contains(&len) { bail!("invalid receive-pack command length"); }
        total += len;
        if total > 1024 * 1024 || updates.len() >= MAX_COMMANDS { bail!("receive-pack command budget exceeded"); }
        let mut data = vec![0u8; len - 4];
        input.read_exact(&mut data)?;
        let text = std::str::from_utf8(&data).context("receive-pack ref names must be UTF-8")?.trim_end_matches('\n');
        let (line, caps) = text.split_once('\0').map_or((text, None), |(line, caps)| (line, Some(caps)));
        if let Some(caps) = caps {
            if !updates.is_empty() { bail!("capabilities are only permitted on the first command"); }
            for cap in caps.split_ascii_whitespace() {
                match cap {
                    "atomic" => bail!("atomic receive-pack is not implemented; no refs or objects were published"),
                    "side-band-64k" => sideband = true,
                    "report-status" | "report-status-v2" | "quiet" | "ofs-delta" | "object-format=sha1" => {},
                    value if value.starts_with("agent=") || value.starts_with("session-id=") => {},
                    _ => bail!("unsupported receive-pack capability: {cap}"),
                }
            }
        }
        let fields: Vec<_> = line.split(' ').collect();
        if fields.len() != 3 { bail!("malformed receive-pack update command"); }
        let old = parse_oid(fields[0])?;
        let new = parse_oid(fields[1])?;
        let name = fields[2];
        if !name.starts_with("refs/heads/") || !grit_lib::check_ref_format::receive_pack_refname_ok(name, new.is_none()) {
            bail!("checked receive-pack currently accepts valid refs/heads/ branch names only");
        }
        if !names.insert(name.to_owned()) { bail!("duplicate receive-pack ref command: {name}"); }
        updates.push(Update { old, new, name: name.to_owned() });
    }
    Ok((updates, sideband))
}

pub(super) fn run(args: Args, repo: Repository, config: ConfigSet) -> Result<()> {
    ensure_supported(&repo, &config)?;
    if args.skip_connectivity_check || args.reject_thin_pack_for_testing {
        bail!("checked receive-pack does not support connectivity bypass or test-only flags");
    }
    let configured_limit = config.get_i64("receive.maxInputSize").transpose().map_err(anyhow::Error::msg)?;
    if configured_limit.is_some_and(|value| value < 0) { bail!("receive.maxInputSize must not be negative"); }
    let limit = configured_limit.filter(|value| *value > 0).map(|value| value as u64).unwrap_or(MAX_PACK).min(MAX_PACK);
    // Reuse the validated full config, including system hide-ref rules. No
    // configured alternate-ref helper is run by this profile.
    super::advertise_refs_with_config(&repo, &HashSet::new(), &config)?;
    let mut input = io::stdin().lock();
    let (updates, sideband) = read_commands(&mut input)?;
    if updates.is_empty() { return Ok(()); }
    let mut pack = Vec::new();
    input.take(limit + 1).read_to_end(&mut pack)?;
    if pack.len() as u64 > limit { bail!("checked receive-pack exceeds its configured or 64 MiB input limit"); }
    let mut diag = DiagSink::new(sideband);
    match execute(&repo, &config, &updates, &pack, &mut diag) {
        Ok(outcomes) => super::write_status_lines(&outcomes, &"0".repeat(40), b"unpack ok\n", &mut diag)?,
        Err(error) => {
            diag.line(&format!("error: {error:#}"));
            let outcomes: Vec<_> = updates.iter().map(|u| RefOutcome::rejected(&u.name, "unpacker error")).collect();
            super::write_status_lines(&outcomes, &"0".repeat(40), b"unpack failed\n", &mut diag)?;
        }
    }
    diag.finish()
}

fn execute(repo: &Repository, config: &ConfigSet, updates: &[Update], pack: &[u8], diag: &mut DiagSink) -> Result<Vec<RefOutcome>> {
    // Only owned quarantine files are removed on failure. Never infer ownership
    // from a before/after directory listing, and never roll back other writers.
    if !pack.is_empty() { validate_pack_budget(pack)?; }
    let base = repo.odb.objects_dir().canonicalize()?;
    let base_text = base.to_str().context("object directory must be UTF-8")?;
    if base_text.contains(['\n', '\r']) { bail!("object directory cannot be encoded as an alternate"); }
    let quarantine = tempfile::Builder::new().prefix("incoming-gitr-").tempdir_in(&base)?;
    std::fs::create_dir(quarantine.path().join("info"))?;
    std::fs::write(quarantine.path().join("info/alternates"), format!("{base_text}\n"))?;
    let mut view = super::open_repo(&repo.git_dir)?;
    view.odb = Odb::new(quarantine.path()).with_config_git_dir(repo.git_dir.clone());
    if !pack.is_empty() {
        grit_lib::unpack_objects::unpack_objects(&mut &pack[..], &view.odb, &grit_lib::unpack_objects::UnpackOptions {
            strict: true, quiet: true, max_input_bytes: Some(MAX_PACK), ..Default::default()
        })?;
    }
    let hidden = grit_lib::hide_refs::hide_ref_patterns_receive(config);
    let deny_deletes = config.get_bool("receive.denyDeletes").transpose().map_err(anyhow::Error::msg)?.unwrap_or(false);
    let deny_nff = config.get_bool("receive.denyNonFastForwards").transpose().map_err(anyhow::Error::msg)?.unwrap_or(false);
    let protected_head = match refs::read_ref_snapshot(&repo.git_dir, "HEAD")? {
        Some(Ref::Symbolic(name)) => Some(name),
        _ => None,
    };
    let mut failures = Vec::with_capacity(updates.len());
    for update in updates {
        let expected = update.old.map(Ref::Direct);
        let reason = if grit_lib::hide_refs::ref_is_hidden(&update.name, &grit_lib::ref_namespace::storage_ref_name(&update.name), &hidden) {
            Some("hidden ref".to_owned())
        } else if refs::read_ref_snapshot(&repo.git_dir, &update.name)?.as_ref() != expected.as_ref() {
            Some("stale expected old value".to_owned())
        } else if update.new.is_none() && deny_deletes {
            Some("deletion prohibited".to_owned())
        } else if update.new.is_none() && protected_head.as_deref() == Some(update.name.as_str()) {
            Some("deletion of the current branch prohibited".to_owned())
        } else if let Some(new) = update.new {
            match view.odb.read(&new) {
                Err(_) => Some("missing necessary objects".to_owned()),
                Ok(object) if update.name.starts_with("refs/heads/") && object.kind != ObjectKind::Commit => Some("branch tip must be a commit".to_owned()),
                Ok(_) => {
                    if !grit_lib::connectivity::push_tip_connected_to_refs(&view, new, &HashSet::new(), None)? {
                        Some("missing necessary objects".to_owned())
                    } else if deny_nff && update.name.starts_with("refs/heads/") && update.old.is_some_and(|old| !grit_lib::merge_base::is_ancestor(&view, old, new).unwrap_or(false)) {
                        Some("non-fast-forward".to_owned())
                    } else { None }
                }
            }
        } else { None };
        failures.push(reason);
    }
    // A concurrently changed hook configuration also fails before publication.
    ensure_supported(repo, &ConfigSet::load(Some(&repo.git_dir), true)?)?;
    if failures.iter().any(Option::is_none) {
        for directory in std::fs::read_dir(quarantine.path())? {
            let directory = directory?;
            let prefix = directory.file_name();
            let Some(prefix) = prefix.to_str() else { bail!("invalid quarantine path"); };
            if prefix == "info" { continue; }
            if prefix.len() != 2 || !prefix.bytes().all(|b| b.is_ascii_hexdigit()) { bail!("unexpected quarantine entry"); }
            for entry in std::fs::read_dir(directory.path())? {
                let entry = entry?;
                let suffix = entry.file_name();
                let suffix = suffix.to_str().context("invalid quarantine object name")?;
                let oid = ObjectId::from_hex(&format!("{prefix}{suffix}"))?;
                let object = view.odb.read(&oid)?;
                let published = repo.odb.write_local(object.kind, &object.data)?;
                if published != oid { bail!("quarantine object identity mismatch"); }
            }
        }
    }
    let mut outcomes = Vec::with_capacity(updates.len());
    for (update, failure) in updates.iter().zip(failures) {
        let reason = if let Some(reason) = failure { Some(reason) } else {
            let expected = update.old.map(Ref::Direct);
            let new = update.new.map(Ref::Direct);
            refs::compare_and_swap_ref(&repo.git_dir, &update.name, expected.as_ref(), new.as_ref()).err().map(|e| {
                diag.line(&format!("error: {}: {e}", update.name));
                "reference changed or could not be published".to_owned()
            })
        };
        outcomes.push(match reason {
            Some(reason) => RefOutcome::rejected(&update.name, &reason),
            None => RefOutcome::accepted(&update.name),
        }.with_delete(update.new.is_none()));
    }
    Ok(outcomes)
}

// Bound allocation before entering upstream's pack/delta decoder. Each zlib
// stream is counted with a small fixed buffer; delta target lengths also count
// toward the total expansion budget. Hash and structural validation still run
// in unpack_objects after this resource preflight.
fn validate_pack_budget(pack: &[u8]) -> Result<()> {
    if pack.len() < 32 || &pack[..4] != b"PACK" { bail!("invalid pack header"); }
    let count = u32::from_be_bytes(pack[8..12].try_into()?);
    if count > MAX_OBJECTS { bail!("checked receive-pack exceeds 10000 objects"); }
    let mut pos = 12usize;
    let mut expanded = 0u64;
    for _ in 0..count {
        let object_offset = pos;
        let mut byte = *pack.get(pos).context("truncated pack entry")?; pos += 1;
        let kind = (byte >> 4) & 7;
        let mut size = u64::from(byte & 15);
        let mut shift = 4u32;
        while byte & 128 != 0 {
            byte = *pack.get(pos).context("truncated pack size")?; pos += 1;
            if shift >= 63 || u64::from(byte & 127) > (u64::MAX >> shift) { bail!("overflowing pack object size"); }
            size |= u64::from(byte & 127) << shift; shift += 7;
        }
        if size > MAX_OBJECT { bail!("checked receive-pack object exceeds 16 MiB"); }
        match kind {
            1..=4 => {},
            6 => {
                let mut distance = 0usize;
                let mut first = true;
                loop {
                    let value = *pack.get(pos).context("truncated delta offset")?; pos += 1;
                    if !first { distance = distance.checked_add(1).and_then(|n| n.checked_mul(128)).context("overflowing delta offset")?; }
                    distance = distance.checked_add(usize::from(value & 127)).context("overflowing delta offset")?;
                    first = false;
                    if value & 128 == 0 { break; }
                }
                if distance == 0 || distance > object_offset { bail!("invalid delta base offset"); }
            },
            7 => { pos = pos.checked_add(20).context("delta offset overflow")?; },
            _ => bail!("invalid packed object kind"),
        }
        let mut decoder = flate2::read::ZlibDecoder::new(pack.get(pos..).context("truncated pack data")?);
        let mut buffer = [0u8; 8192];
        let mut delta = Vec::new();
        let mut output = 0u64;
        loop {
            let n = decoder.read(&mut buffer)?;
            if n == 0 { break; }
            output += n as u64;
            if output > size || output > MAX_OBJECT { bail!("pack object exceeds declared size"); }
            if kind == 6 || kind == 7 { delta.extend_from_slice(&buffer[..n]); }
        }
        if output != size { bail!("pack object has wrong declared size"); }
        let consumed = usize::try_from(decoder.total_in())?;
        if consumed == 0 { bail!("empty compressed object stream"); }
        pos = pos.checked_add(consumed).context("pack offset overflow")?;
        expanded = expanded.checked_add(size).context("pack expansion overflow")?;
        if kind == 6 || kind == 7 {
            let result = validate_delta_budget(&delta)?;
            expanded = expanded.checked_add(result).context("delta expansion overflow")?;
        }
        if expanded > MAX_EXPANDED { bail!("checked receive-pack expansion exceeds 128 MiB"); }
    }
    if pos.checked_add(20) != Some(pack.len()) { bail!("pack trailer length mismatch"); }
    Ok(())
}

fn validate_delta_budget(delta: &[u8]) -> Result<u64> {
    let mut pos = 0;
    let base = delta_size(delta, &mut pos)?;
    let target = delta_size(delta, &mut pos)?;
    if base > MAX_OBJECT || target > MAX_OBJECT { bail!("delta base or result exceeds 16 MiB"); }
    let mut produced = 0u64;
    while pos < delta.len() {
        let command = delta[pos]; pos += 1;
        if command == 0 { bail!("invalid delta instruction"); }
        let count = if command & 128 != 0 {
            let mut offset = 0u64;
            let mut count = 0u64;
            for bit in 0..7 {
                if command & (1 << bit) != 0 {
                    let byte = *delta.get(pos).context("truncated delta instruction")?; pos += 1;
                    if bit < 4 { offset |= u64::from(byte) << (8 * bit); }
                    else { count |= u64::from(byte) << (8 * (bit - 4)); }
                }
            }
            if count == 0 { count = 65536; }
            if offset.checked_add(count).is_none_or(|end| end > base) { bail!("delta copy exceeds base"); }
            count
        } else {
            let count = usize::from(command);
            pos = pos.checked_add(count).context("delta instruction overflow")?;
            if pos > delta.len() { bail!("truncated delta insertion"); }
            count as u64
        };
        produced = produced.checked_add(count).context("delta result overflow")?;
        if produced > target { bail!("delta instruction exceeds declared result size"); }
    }
    if produced != target { bail!("delta result size mismatch"); }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(value: &[u8]) -> Vec<u8> {
        let mut bytes = format!("{:04x}", value.len() + 4).into_bytes();
        bytes.extend_from_slice(value); bytes.extend_from_slice(b"0000"); bytes
    }
    #[test]
    fn command_parser_rejects_lossy_truncated_duplicate_and_atomic_input() {
        let zero = "0".repeat(40);
        let valid = format!("{zero} {} refs/heads/main\0report-status", "1".repeat(40));
        assert_eq!(read_commands(&mut &packet(valid.as_bytes())[..]).unwrap().0.len(), 1);
        for bad in [format!("{zero} 123 refs/heads/main"), format!("{zero} {} refs/heads/main\0atomic", "1".repeat(40))] {
            assert!(read_commands(&mut &packet(bad.as_bytes())[..]).is_err());
        }
        let mut bytes = format!("{zero} {} refs/heads/", "1".repeat(40)).into_bytes(); bytes.push(255);
        assert!(read_commands(&mut &packet(&bytes)[..]).is_err());
        assert!(read_commands(&mut &b"001"[..]).is_err());
        let line = format!("{zero} {} refs/heads/main", "1".repeat(40));
        let mut duplicate = packet(line.as_bytes()); duplicate.truncate(duplicate.len() - 4);
        duplicate.extend(packet(line.as_bytes()));
        assert!(read_commands(&mut &duplicate[..]).is_err());
    }
    #[test]
    fn delta_preflight_rejects_expansion_beyond_declared_target() {
        assert_eq!(validate_delta_budget(&[1, 1, 1, b'x']).unwrap(), 1);
        assert!(validate_delta_budget(&[1, 1, 2, b'x', b'y']).is_err());
        assert!(validate_delta_budget(&[0x80, 0x80, 4, 1, 0x80]).is_err());
    }
}

fn delta_size(data: &[u8], offset: &mut usize) -> Result<u64> {
    let mut size = 0u64;
    for shift in (0..63).step_by(7) {
        let byte = *data.get(*offset).context("truncated delta size")?; *offset += 1;
        size |= u64::from(byte & 127) << shift;
        if byte & 128 == 0 { return Ok(size); }
    }
    bail!("overflowing delta size")
}
