# Independent scoped licensing review for gitr

Research date: 2026-11-08, America/Denver. Repository evidence pinned to `69a16ea1506d8efa20dff0e559b5716dc95743bd`.

## Recommendation

Record the user's authorization to choose a better license for code they control. **Do not change the existing Rust or root license declarations solely because the 12 visible Rust-path commits share one GitHub author.** Complete a file-level provenance and build-composition audit first. This is a concrete evidence gap, not a request for additional permission.

For **new, independently authored Rust components**, use **MIT OR Apache-2.0** as the flexible default. Keep third-party source under its actual license and preserve required notices. If a selected GPLv3 donor materially improves the product, a GPLv3 combined distribution can be evaluated without requiring every original reusable library to become GPLv3-only. Do not label such a combined product wholly MIT/Apache.

## Evidence from the current repository

1. Root [COPYING](https://github.com/glopyglerky/gitr/blob/69a16ea1506d8efa20dff0e559b5716dc95743bd/COPYING) explicitly selects GPL version 2 for inherited Git, unless a file states otherwise. Its introduction explains that moving inherited code to GPLv3 would require the relevant copyright holders' authorization.
2. [rust/gitr/Cargo.toml](https://github.com/glopyglerky/gitr/blob/69a16ea1506d8efa20dff0e559b5716dc95743bd/rust/gitr/Cargo.toml) explicitly declares `GPL-2.0-only`. It also creates its own workspace and says the Rust port is separate from upstream Git's C-linked `gitcore` crate.
3. The [path-scoped commit history](https://api.github.com/repos/glopyglerky/gitr/commits?sha=69a16ea1506d8efa20dff0e559b5716dc95743bd&path=rust/gitr&per_page=100) returns 12 commits attributed to `glopyglerky`. This identifies commit authorship, not all copyright owners or the origin of every source fragment, fixture, translated algorithm implementation, or imported file.
4. The inspected crate directory has no local license text that independently documents a different grant. Its manifest and inherited root notice therefore cannot be casually replaced with an assumption of independent authorship.

The existing workspace separation is useful evidence for an independently built Rust product. It does not by itself prove that its source is independently authored. Repository size, a directory boundary, and absence of a C library in the direct manifest are insufficient to determine all source-origin and distribution questions.

## Primary licensing guidance

GNU's FAQ states that a copyright holder may offer their own code under different nonexclusive licenses. The same FAQ explains that another contributor's GPL-covered contributions require that contributor's permission before relicensing. This supports a scoped relicensing path where ownership is established; it does not authorize removing rights from upstream Git. [GNU FAQ, ReleaseUnderGPLAndNF and Consider](https://www.gnu.org/licenses/gpl-faq.en.html#ReleaseUnderGPLAndNF).

The Apache Software Foundation states that Apache-2.0 software can be included in a GPLv3 project, while Apache-2.0 and GPLv2 are incompatible. Therefore Apache-only donors should remain candidates for the independent Rust product, but must not be presented as distributable inside a GPL-2.0-only combined program merely by changing one Cargo field. [ASF compatibility guidance](https://www.apache.org/licenses/GPL-compatibility.html).

GPLv2 distinguishes independent works distributed separately or as an aggregate from a combined derivative work. Its text also includes translation into another language within modification. A Rust translation of protected Git code cannot be assumed independent just because the language changed. The GNU FAQ says communication mechanisms and semantics both matter when distinguishing separate programs from one combined program. [GPLv2 sections 0 and 2, as included in gitr](https://github.com/glopyglerky/gitr/blob/69a16ea1506d8efa20dff0e559b5716dc95743bd/COPYING); [GNU FAQ, MereAggregation](https://www.gnu.org/licenses/gpl-faq.en.html#MereAggregation).

## Scope decisions

| Material | Working disposition |
| --- | --- |
| Inherited Git source and modified derivatives | Retain GPL-2.0-only unless a specific file or additional grant authorizes another route |
| Existing authored Rust with confirmed sole ownership and independent origin | May be offered under a new parallel license under the user's authorization; record exactly which files and evidence |
| New independently authored Rust | Prefer MIT OR Apache-2.0 for reusable components |
| MIT/Apache dual-licensed donors | Preserve both notices; choose a compatible offered route for each combined distribution |
| Apache-2.0-only donors | Eligible for independent permissive Rust or compatible GPLv3 combinations, not an unchanged GPLv2-only combined program |
| GPLv3 donors | Assess a GPLv3 product/module distribution where compatible; do not assume Git GPLv2-only can be converted by association |
| No-license, source-available, FSL/BUSL, or custom-rider code | Follow actual upstream permissions; changing gitr's own license does not erase those terms |

## Minimum audit needed before changing existing declarations

For each current Rust source/test/helper file, record the originating commit, whether it is newly authored or imported/translated, any upstream source and license, and the basis for the project's relicensing authority. Review the initial import and subsequent copy/import operations, not just the last author in blame. A similarity search can find concerns but cannot prove independent origin by itself.

Then inspect the resolved dependency/build graph and release packaging. Confirm whether the Rust executable links inherited GPLv2-only code, incorporates translated portions, embeds third-party fixtures, or merely interoperates with separately distributed programs. Preserve the inherited Git notices and represent the repository as mixed-license if that is the result.

Once that evidence is recorded, change only the identified scope and add matching license texts, per-file SPDX identifiers or an equivalent clear scope map, and third-party notices. The user has already authorized the lawful choice; no second confirmation is needed simply to perform that audit.

This assessment recommends an engineering path based on the retrieved license texts and observed repository structure. It does not claim that this limited inspection establishes every copyright holder or conclusively settles derivative-work status.

