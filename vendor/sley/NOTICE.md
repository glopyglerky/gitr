# Sley import provenance

Upstream: https://github.com/HeddleCo/sley

Revision: `a71654f50de24b3c750f0df5f1829db7342491f6`, workspace 0.13.0.

Sley declares Apache-2.0 in Cargo.toml and README.md. This import preserves
those declarations and all existing source notices. Upstream has no separate
license-text file at this revision; LICENSE-APACHE supplies the standard
Apache License 2.0 text. This notice does not assert new upstream copyright
ownership or change any source grant.

Gitr's imported-file manifest is ../../Documentation/gitr/sley-import.json.
It records original Git blob IDs, original SHA-256 hashes, installed hashes,
local changes, added files, and exclusions. Local modifications retain the
imported file's upstream license. Independently authored root entrypoints use
Gitr's separate MIT OR Apache-2.0 grant.

The source is adapted for the Gitr frontend, checked ref publication, repack
failure handling, byte-preserving staging, grep pattern files, and native upload-archive. The
integration record describes tested behavior and remaining gaps. Upstream
small shell dispatch adapters and generated test fixtures are retained explicitly; this is not an assertion
that every program embedded in a Rust file is implemented in Rust.

The next Gitr adaptation adds a streaming remote archive client through the
existing SSH/Git transports, checks explicit service termination, and repairs
symbolic revision-selector rendering and symbolic-ref enumeration. The added
Rust module and regression tests are independently authored Gitr code under
MIT OR Apache-2.0. Generated shell fixtures model explicitly configured test
services; no copied filter-branch or i18n shell implementation was restored.

The historical Grit donor is outside the active executable's build graph.

Two upstream embedded programs were excluded after source-origin review:
`commands/filter_branch.rs` contained a shell engine with substantial overlap
with Git's git-filter-branch.sh, and sley-i18n's GIT_SH_I18N contained substantial
overlap with Git's git-sh-i18n.sh. The donor copies omitted Git's original
attribution headers. Gitr does not claim the Apache workspace declaration
relicenses those inherited fragments. The product refuses filter-branch before
mutation and does not materialize the shell i18n library. Their upstream Git
blob identities remain recorded for future provenance resolution.
