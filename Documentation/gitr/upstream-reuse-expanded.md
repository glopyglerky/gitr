# Expanded upstream reuse review

The [expanded review and import plan](research/2026-11-08-upstream-survey/expansion/README.md)
extends the 476-candidate survey with twelve pinned follow-up reviews, specific
whole-crate/fork boundaries, implementation findings and acceptance tests.

The combined catalog contains **483 candidates: seven new repositories and five
reassessed baseline entries**. The original assessments remain intact alongside
the new evidence. [Observed qualification and exact counts](research/2026-11-08-upstream-survey/expansion/qualification.json)
record successful run 42547007431, ten catalog self-tests, the baseline integrity
check, generated files and the downloaded artifact's verified SHA-256. This is
metadata qualification, not donor runtime qualification.

[Structured follow-up records](research/2026-11-08-upstream-survey/expansion/reviews.json)
retain exact commits, inspected paths, license evidence and unresolved obligations.
The original catalogs are preserved. From the repository root:

```sh
python3 ci/gitr-upstream-review.py --self-test
python3 ci/gitr-upstream-review.py --output /tmp/gitr-expanded-survey
```

This generates the complete deduplicated CSV/JSON and coverage report. The
read-only [gitr upstream review workflow](../../.github/workflows/gitr-upstream-review.yml)
also retains them as the `gitr-expanded-upstream-survey` artifact. Its original
[qualified run](https://github.com/glopyglerky/gitr/actions/runs/42547007431) contains
[the combined catalog archive](https://github.com/glopyglerky/gitr/actions/runs/42547007431/artifacts/13643443972).
Artifacts are subject to GitHub retention; the committed inputs and generator
preserve reproducibility after an artifact expires.

Complete code copies, coherent package-family imports and whole application forks
are eligible when rights and integration economics allow them. redb, Gitoxide,
Wasmi, Lustr, Git interoperability and existing durable data remain the foundation.
No donor code, runtime dependencies, license declarations or repository databases
are changed by this research increment.
