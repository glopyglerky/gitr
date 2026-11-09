# Expanded upstream reuse review

The [expanded review and import plan](research/2026-11-08-upstream-survey/expansion/README.md)
extends the 476-candidate survey with twelve pinned follow-up reviews, specific
whole-crate/fork boundaries, implementation findings and acceptance tests.

[Structured follow-up records](research/2026-11-08-upstream-survey/expansion/reviews.json)
retain exact commits, inspected paths, license evidence and unresolved obligations.
The original catalogs are preserved. Run `python3 ci/gitr-upstream-review.py --output
/tmp/gitr-expanded-survey` from the repository root to generate the deduplicated
combined CSV/JSON and coverage report; the read-only `gitr upstream review`
workflow also retains them as an artifact.

Complete code copies, coherent package-family imports and whole application forks
are eligible when rights and integration economics allow them. redb, Gitoxide,
Wasmi, Lustr, Git interoperability and existing durable data remain the foundation.
No donor code, runtime dependencies, license declarations or repository databases
are changed by this research increment.
