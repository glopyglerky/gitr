//! Plumbing operations — low-level, machine-stable building blocks that map
//! ~1:1 to a Git plumbing command (e.g. `rev-list`, `diff-tree`, `merge-tree`,
//! `ls-files`, `cat-file`).
//!
//! These re-export the existing implementation modules under a curated,
//! navigable namespace; the flat module paths (`grit_lib::rev_list`, …) remain
//! valid, so this layer is purely additive. Output produced here is stable and
//! intended for scripts/other programs, never coloured or tty-aware.
//!
//! Curated re-exports are populated in the public-API curation phase; this
//! module is the scaffolding the porcelain layer and the CLI build upon.
