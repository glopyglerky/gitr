//! Metadata-only game asset inventory. Never follows symlinks or reads assets.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct Asset {
    pub path: String,
    pub bytes: u64,
    pub category: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Scan {
    pub files: u64,
    pub total_bytes: u64,
    pub large_files: u64,
    pub threshold_bytes: u64,
    pub omitted_large_files: u64,
    pub largest: Vec<Asset>,
}

pub fn scan(root: &Path, threshold: u64, limit: usize) -> Result<Scan> {
    ensure!(root.is_dir(), "asset scan requires a worktree directory");
    ensure!(
        (1..=10_000).contains(&limit),
        "asset result limit must be between 1 and 10000"
    );
    let mut largest = BinaryHeap::new();
    let mut report = Scan {
        files: 0,
        total_bytes: 0,
        large_files: 0,
        threshold_bytes: threshold,
        omitted_large_files: 0,
        largest: Vec::new(),
    };
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    for entry in walker {
        let entry = entry.context("asset scan could not enumerate a path")?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let meta = entry
            .metadata()
            .context("asset metadata changed during scan")?;
        report.files += 1;
        report.total_bytes = report
            .total_bytes
            .checked_add(meta.len())
            .context("asset byte count overflow")?;
        if meta.len() < threshold {
            continue;
        }
        report.large_files += 1;
        largest.push(Reverse((
            meta.len(),
            entry.path().strip_prefix(root)?.to_path_buf(),
        )));
        if largest.len() > limit {
            largest.pop();
        }
    }
    let mut ranked: Vec<(u64, PathBuf)> = largest.into_iter().map(|entry| entry.0).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    report.omitted_large_files = report.large_files - ranked.len() as u64;
    report.largest = ranked
        .into_iter()
        .map(|(bytes, path)| Asset {
            category: category(&path),
            path: path.to_string_lossy().into_owned(),
            bytes,
        })
        .collect();
    Ok(report)
}

fn category(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "uasset" | "umap" => "unreal-binary",
        "unity" | "prefab" | "asset" | "meta" => "unity-review-serialization",
        "blend" | "fbx" | "psd" | "exr" | "glb" => "source-art",
        "wav" | "flac" | "mp4" | "mov" | "mp3" | "ogg" => "media",
        "zip" | "7z" | "pak" | "iso" => "archive-or-build-output",
        _ => "large-file",
    }
}

pub fn attributes(engine: &str) -> Result<String> {
    let mut output = String::from(
        "# Review and append selected rules to .gitattributes.\n# These rules require Git LFS transfers and server-side locks where supported.\n",
    );
    match engine {
        "unreal" => {
            output.push_str("# Unreal package/map assets are binary and normally need exclusive editing.\n*.uasset filter=lfs diff=lfs merge=lfs -text lockable\n*.umap filter=lfs diff=lfs merge=lfs -text lockable\n");
        }
        "unity" => {
            output.push_str("# Keep Unity .meta files with their assets.\n# Keep Force Text .unity, .prefab and .asset files in normal Git; configure UnityYAMLMerge separately.\n");
        }
        "generic" => {}
        _ => anyhow::bail!("unknown game profile; use unreal, unity or generic"),
    }
    for ext in [
        "blend", "fbx", "psd", "exr", "glb", "wav", "flac", "mp4", "mov",
    ] {
        output.push_str(&format!(
            "*.{ext} filter=lfs diff=lfs merge=lfs -text lockable\n"
        ));
    }
    Ok(output)
}
