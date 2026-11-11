# Rit source licenses

Rit-owned original entrypoints, tests and build helpers use MIT OR Apache-2.0.
The full texts are in the root `LICENSE-MIT` and `LICENSE-APACHE` files.
The root `LICENSE` defines the contribution scope. Imported code keeps its
original terms when it is moved or modified.

| Source | Terms and source record |
| --- | --- |
| Original `src`, `tests`, `ci`, `examples` and `xtask` work | MIT OR Apache-2.0, subject to existing file notices |
| `vendor/sley` | Apache-2.0; exact file origins, local changes and hashes in `sley-import.json` |
| `compat/git` | Upstream Git's `COPYING`, `LGPL-2.1` and individual file notices at the pinned Git commit |
| Inherited Git documents and notices | Their existing grants and copyright notices |
| Other dependencies | Their own declared grants and notices |

The Sley import preserves its workspace license declaration and supplies the
standard Apache text with a provenance notice. See
[the Sley notice](../../vendor/sley/NOTICE.md).

The compatibility build produces a separate upstream Git installation. Retain
its complete source connection, license texts, installed helpers and templates.
A process boundary does not relicense either program. Distribution must satisfy
the terms of every included component. Do not label the whole distribution as
exclusively MIT, Apache or Rust.

Before importing another donor, record its exact revision, source paths,
original hashes, exclusions, local adaptations, notices, resolved dependencies
and applicable distribution terms. A fork's project license does not replace
the licenses on inherited files. Earlier public grants remain in effect.
