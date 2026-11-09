//! Copy bundled `upstream_help_synopsis.rs` into `OUT_DIR` for t0450 help parity.
//! Gitr deliberately does not install upstream shell helper programs.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let dst = out_dir.join("upstream_help_synopsis.rs");
    let bundled = manifest_dir.join("upstream_help_synopsis.rs");

    fs::copy(&bundled, &dst).unwrap_or_else(|e| {
        panic!(
            "copy bundled {} to {}: {e}",
            bundled.display(),
            dst.display()
        );
    });
    println!("cargo:rerun-if-changed={}", bundled.display());

}

