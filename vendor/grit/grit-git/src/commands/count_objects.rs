//! `grit count-objects` command.

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use grit_lib::pack::{
    collect_local_pack_info, read_alternates_recursive, read_local_pack_indexes, read_pack_index,
};
use grit_lib::repo::Repository;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Arguments for `grit count-objects`.
#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Verbose breakdown.
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,
}

/// Run `grit count-objects`.
pub fn run(args: Args) -> Result<()> {
    let repo = Repository::discover(None).context("failed to discover repository")?;
    let objects_dir = repo.git_dir.join("objects");

    // Match Git: only count loose objects in this repository's primary object
    // store, not objects reachable only via `info/alternates`.
    let (loose_count, loose_size, loose_ids) = scan_loose_objects(&objects_dir)?;
    if !args.verbose {
        println!("{} objects, {} kilobytes", loose_count, loose_size / 1024);
        return Ok(());
    }

    let mut pack_info = collect_local_pack_info(&objects_dir)?;
    adjust_pack_info_for_alternates(&objects_dir, &mut pack_info)?;
    let prune_packable = loose_ids.intersection(&pack_info.object_ids).count();
    let (garbage_count, garbage_size) = scan_pack_garbage(&objects_dir)?;
    let alternates = read_alternates_recursive(&objects_dir)?;

    println!("count: {loose_count}");
    println!("size: {}", loose_size / 1024);
    println!("in-pack: {}", pack_info.object_count);
    println!("packs: {}", pack_info.pack_count);
    println!("size-pack: {}", pack_info.size_bytes / 1024);
    println!("prune-packable: {prune_packable}");
    println!("garbage: {garbage_count}");
    println!("size-garbage: {}", garbage_size / 1024);
    for alt in alternates {
        println!("alternate: {}", alt.display());
    }
    Ok(())
}

fn scan_loose_objects(
    objects_dir: &Path,
) -> Result<(usize, u64, HashSet<grit_lib::objects::ObjectId>)> {
    let mut count = 0usize;
    let mut size = 0u64;
    let mut ids = HashSet::new();
    let rd = fs::read_dir(objects_dir).map_err(anyhow::Error::from)?;
    for entry in rd {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "pack" || name == "info" || name.len() != 2 {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let sub = fs::read_dir(&path)?;
        for file in sub {
            let file = file?;
            let file_name = file.file_name().to_string_lossy().to_string();
            if file_name.len() != 38 {
                continue;
            }
            let obj_path = file.path();
            let meta = match fs::metadata(&obj_path) {
                Ok(meta) => meta,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err.into()),
            };
            if !meta.is_file() {
                continue;
            }
            let hex = format!("{name}{file_name}");
            if hex == "4b825dc642cb6eb9a060e54bf8d69288fbee4904" {
                continue;
            }
            if let Ok(oid) = hex.parse() {
                ids.insert(oid);
            }
            count += 1;
            size += meta.len();
        }
    }
    Ok((count, size, ids))
}

fn adjust_pack_info_for_alternates(
    objects_dir: &Path,
    info: &mut grit_lib::pack::LocalPackInfo,
) -> Result<()> {
    let alternates = read_alternates_recursive(objects_dir)?;
    if alternates.is_empty() {
        return Ok(());
    }

    let indexes = read_local_pack_indexes(objects_dir)?;
    let mut kept_ids = HashSet::new();
    let mut kept_count = 0usize;
    let mut kept_packs = 0usize;
    let mut kept_size = 0u64;
    for idx in indexes {
        let mut ids = Vec::new();
        let mut all_alternate = true;
        for entry in &idx.entries {
            if entry.oid.len() != 20 {
                all_alternate = false;
                continue;
            }
            let Ok(oid) = grit_lib::objects::ObjectId::from_bytes(&entry.oid) else {
                all_alternate = false;
                continue;
            };
            if !object_exists_in_alternates(&alternates, &oid) {
                all_alternate = false;
            }
            ids.push(oid);
        }
        if all_alternate {
            continue;
        }
        kept_packs += 1;
        kept_count += ids.len();
        kept_ids.extend(ids);
        kept_size = kept_size
            .saturating_add(fs::metadata(&idx.pack_path).map(|m| m.len()).unwrap_or(0))
            .saturating_add(fs::metadata(&idx.idx_path).map(|m| m.len()).unwrap_or(0));
    }
    info.pack_count = kept_packs;
    info.object_count = kept_count;
    info.object_ids = kept_ids;
    info.size_bytes = kept_size;
    Ok(())
}

fn object_exists_in_alternates(alternates: &[PathBuf], oid: &grit_lib::objects::ObjectId) -> bool {
    alternates.iter().any(|objects_dir| {
        objects_dir
            .join(oid.loose_prefix())
            .join(oid.loose_suffix())
            .is_file()
            || read_local_pack_indexes(objects_dir)
                .map(|indexes| indexes.iter().any(|idx| idx.contains(oid)))
                .unwrap_or(false)
    })
}

fn scan_pack_garbage(objects_dir: &Path) -> Result<(usize, u64)> {
    let pack_dir = objects_dir.join("pack");
    let rd = match fs::read_dir(&pack_dir) {
        Ok(rd) => rd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok((0, 0)),
        Err(err) => return Err(err.into()),
    };

    #[derive(Default)]
    struct PackStemFiles {
        pack: Option<std::path::PathBuf>,
        idx: Option<std::path::PathBuf>,
        keep: Option<std::path::PathBuf>,
        invalid_idx: bool,
    }

    let mut pack_by_stem: BTreeMap<String, PackStemFiles> = BTreeMap::new();
    let mut garbage_count = 0usize;
    let mut garbage_size = 0u64;

    for entry in rd {
        let entry = entry?;
        let path = entry.path();
        let meta = fs::metadata(&path)?;
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();

        match ext {
            "pack" => {
                pack_by_stem.entry(stem).or_default().pack = Some(path.clone());
            }
            "idx" => {
                let files = pack_by_stem.entry(stem).or_default();
                if let Err(err) = read_pack_index(&path) {
                    let msg = err
                        .to_string()
                        .replace(&path.display().to_string(), &display_git_path(&path));
                    eprintln!("{msg}");
                    files.invalid_idx = true;
                }
                files.idx = Some(path.clone());
            }
            "keep" => {
                pack_by_stem.entry(stem).or_default().keep = Some(path.clone());
            }
            "bitmap" | "rev" | "mtimes" | "promisor" | "midx" => {}
            _ => {
                eprintln!("warning: garbage found: {}", display_git_path(&path));
                garbage_count += 1;
                garbage_size += meta.len();
            }
        }

        if !meta.is_file() {
            garbage_count += 1;
            garbage_size += meta.len();
        }
    }

    for (_stem, files) in pack_by_stem {
        match (&files.pack, &files.idx, &files.keep) {
            (Some(pack), None, Some(keep)) => {
                eprintln!("warning: no corresponding .idx: {}", display_git_path(keep));
                eprintln!("warning: no corresponding .idx: {}", display_git_path(pack));
                garbage_count += 1;
            }
            (Some(pack), None, None) => {
                eprintln!("warning: no corresponding .idx: {}", display_git_path(pack));
                garbage_count += 1;
            }
            (None, Some(idx), Some(keep)) => {
                if !files.invalid_idx {
                    eprintln!(
                        "warning: no corresponding .idx or .pack: {}",
                        display_git_path(keep)
                    );
                    eprintln!("warning: no corresponding .pack: {}", display_git_path(idx));
                    garbage_count += 1;
                }
            }
            (None, Some(idx), None) => {
                eprintln!("warning: no corresponding .pack: {}", display_git_path(idx));
                garbage_count += 1;
            }
            (None, None, Some(keep)) => {
                eprintln!(
                    "warning: no corresponding .idx or .pack: {}",
                    display_git_path(keep)
                );
            }
            _ => {}
        }
    }

    Ok((garbage_count, garbage_size))
}

fn display_git_path(path: &Path) -> String {
    let parts: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect();
    if let Some(pos) = parts.iter().position(|part| part == ".git") {
        return parts[pos..].join("/");
    }
    path.display().to_string()
}
