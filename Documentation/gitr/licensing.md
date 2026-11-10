# Rit licensing and upstream imports

The project is now named Rit. Historical decisions below retain the Gitr name
used at their recorded revisions; the rename changes no license grant.

## Current Sley composition

The active root now links the pinned Apache-2.0 Sley engine and frontend. It no
longer links the historical GPL-2.0-only Grit CLI. This is an actual dependency
composition change, not a relabeling of inherited code. Rit-owned entrypoints
remain MIT OR Apache-2.0, and Sley files and local adaptations remain Apache-2.0.
All dependency obligations still apply. The old Grit and Git source material
retains its original terms in the repository and history.

Sley declares Apache-2.0 in its Cargo workspace and README but supplies no
license-text file at the imported commit. The import includes the standard
Apache-2.0 text as `vendor/sley/LICENSE-APACHE`, preserves those declarations,
and records its provenance in `vendor/sley/NOTICE.md`. No upstream copyright
notice has been invented. See [Sley integration](sley-integration.md).

**The remainder records the prior license decision at its original baseline.
References below to the current GPL-only executable describe that baseline,
not the new Sley executable.**

## Historical decision

Decision date: 2026-11-10. Source baseline:
[`12fd77f5ed2e3f3ed6cc983c52ad7aa452ccd521`](https://github.com/glopyglerky/rit/tree/12fd77f5ed2e3f3ed6cc983c52ad7aa452ccd521).

## Effective change

The repository owner authorized changing Gitr's license to support the best
upstream implementation choices. The owner's contributions in the reviewed
scope are now additionally available under **MIT OR Apache-2.0**. Recipients
choose either license. The full grants are in [LICENSE-MIT](../../LICENSE-MIT)
and [LICENSE-APACHE](../../LICENSE-APACHE); [LICENSE](../../LICENSE) defines
their scope and the current combined executable's obligations.

This change applies to existing identified contributions, not just future
code. [license-scope.json](license-scope.json) records the baseline blobs,
historical source paths and originating commits used to identify that scope.
The grant covers rights controlled by the owner. It does not infer ownership
of imported or translated material merely from a commit's author, its language,
or its directory. Existing third-party notices and obligations still apply.

The independently implemented `crates/action-example` changes its package
license from GPL-2.0-only to MIT OR Apache-2.0. `xtask` and
`crates/test-support` already declare that dual license. The root package
uses Cargo's `license-file = "LICENSE"` because a single permissive label
would misdescribe its current combined build. No runtime dependency, source
algorithm, object format or database is changed by this decision.

## Source and distribution map

| Scope | Applicable terms |
| --- | --- |
| Reviewed Gitr-controlled contributions listed in `license-scope.json` | MIT OR Apache-2.0, in addition to previously granted licenses |
| New independent Gitr implementation, tests and helpers | MIT OR Apache-2.0 by default; record the grant in the source or package |
| Historical `vendor/grit/grit-git`, its embedded Git help, and local adaptations | GPL-2.0-only and preserved upstream notices |
| `vendor/grit/grit-lib`, `vendor/grit/grit-test-support` | Existing MIT grants and notices |
| `vendor/rbit-workflow` | Existing MIT OR Apache-2.0 grant and notices; select MIT in the current combined executable |
| Inherited Git documentation, license texts and recoverable historical source | Existing COPYING and file-specific terms |
| `Documentation/gitr/archive/` and all other donor material | Their existing grants; excluded from the new project grant |
| Current default Gitr executable | Combined GPL-2.0-only work with applicable permissive-license notices retained |

The original Git and Grit license texts remain byte-for-byte unchanged. Old
research and qualification records describe their original revisions; this
document governs the current project contribution grant.

## Why the executable still has a GPLv2-only constraint

The baseline [root manifest](https://github.com/glopyglerky/rit/blob/12fd77f5ed2e3f3ed6cc983c52ad7aa452ccd521/Cargo.toml#L21-L27)
unconditionally depends on `grit-git`, and
[`src/main.rs`](https://github.com/glopyglerky/rit/blob/12fd77f5ed2e3f3ed6cc983c52ad7aa452ccd521/src/main.rs#L188)
calls its Rust entry point in the same process. The `--isolated-git` option
selects a runtime path, not a separate link composition.

Historical Grit's [original split license](https://github.com/gitbutlerapp/grit/blob/e197103087b1110db2d629c4c269ef9258508169/LICENSE)
and [CLI manifest](https://github.com/gitbutlerapp/grit/blob/e197103087b1110db2d629c4c269ef9258508169/grit-git/Cargo.toml)
identify the CLI as GPL-2.0-only. Git's
[project-specific COPYING notice](https://github.com/git/git/blob/v2.56.0/COPYING#L1-L18)
also selects GPLv2 only. Its discussion of a possible future migration and the
license appendix's sample "or later" notice do not grant a blanket upgrade.
Modern Grit's MIT license covers its current source; it does not establish a
new grant for the deleted historical compatibility CLI.

The [Apache Software Foundation](https://www.apache.org/licenses/GPL-compatibility)
and [GNU license guidance](https://www.gnu.org/licenses/license-list.html#apache2)
identify Apache-2.0 as compatible with GPLv3, but not GPLv2-only. Changing
Gitr's own contributions to a permissive license cannot change that result
for somebody else's inherited code.

## Import policy

Choose upstream code for correctness, completeness and maintainability. The
project-owned grant does not require every donor to adopt MIT or Apache, and
the existing GPLv2 build is not a permanent product-license requirement.
Whole crates and coherent package families remain valid import units.

Before adding a donor to a shipping build, record its exact revision, paths,
original hashes, notices, chosen license alternatives, adaptations and resolved
dependency closure. State the resulting combined license. Preserve imported
tests under their own terms as well as implementation source.

| Candidate composition | Required action |
| --- | --- |
| Current CLI with MIT/BSD/ISC or eligible dual-licensed code using its MIT option | Preserve notices and comply with the existing combined GPLv2 obligations |
| Current CLI with Apache-2.0-only, GPLv3-only or AGPLv3-only code | Do not link or represent this as a compatible release; first resolve the inherited GPLv2-only dependency |
| Gitr-controlled code with modern MIT Grit and Apache-only modules, without GPLv2-only material | Eligible for an appropriately reviewed permissive composition |
| GPLv3 or AGPL product composition | Eligible when every incorporated component permits it; project-owned code can participate through its MIT option |
| Genuinely separate programs in one repository or distribution | Assess each program and the nature of their interaction; a process or directory boundary alone does not settle compatibility |
| No-license, proprietary, source-available or custom-restriction code | Obtain and follow the actual permissions; this project grant supplies none on the donor's behalf |

For Apache-only candidates such as Sley, a compatible route requires replacing
or removing the GPLv2-only CLI from that executable, obtaining sufficient
additional upstream permission, or establishing genuinely independent programs.
An isolated evaluation may inform that decision, but is not permission to ship
an incompatible combined executable. Do not weaken Git compatibility merely to
change a license label.

## Maintaining the scope

New independently authored reusable Rit code should declare
`SPDX-License-Identifier: MIT OR Apache-2.0`, or use an equivalent explicit
package/scope declaration. Contributors must have authority to make the stated
grant. Preserve a donor's actual license when modifying imported material;
do not apply the project default indiscriminately to a vendor tree.

The scope inventory records the pre-change source baseline so the audit remains
reproducible. A changed comment or manifest does not rewrite that historical
blob. Future provenance discoveries must narrow/correct the affected scope and
retain any additional terms rather than silently relabeling upstream code.

Cargo documents `license-file` for cases not represented by a single standard
package license, and distinguishes the choice expressed by `OR` from the
simultaneous obligations expressed by `AND`:
[Cargo manifest reference](https://doc.rust-lang.org/cargo/reference/manifest.html#the-license-and-license-file-fields).
The [GNU FAQ](https://www.gnu.org/licenses/gpl-faq.html#ReleaseUnderGPLAndNF)
explains that a copyright holder can offer their own work under multiple
nonexclusive licenses. This decision grants that choice only within the owner's
authority and retains all existing donor obligations.
