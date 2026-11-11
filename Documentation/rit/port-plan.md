# Git port and compatibility replacement

Rit's public project is a complete Git implementation whose source becomes Rust
incrementally. Reuse coherent upstream Rust crates, their tests and their notices.
Preserve upstream Git as a connected source component for behavior that has not
yet been qualified in Rust. The current native implementation is incomplete.

## Working components

| Component | Ownership and required evidence |
| --- | --- |
| Imported Sley engine and CLI | Git algorithms, object formats, refs, worktrees, commands and transports; exact imported source inventory |
| Connected upstream Git | Remaining compatibility behavior and reference tests; immutable source revision and reproducible build |
| Rit entrypoints | Pre-execution backend selection, raw argument forwarding and explicit backend diagnostics |
| Lustr and recorded compiler | Build Rust replacements through the actual supported compiler route and retain its identity |

At this stage every default invocation selects connected Git. Native mode remains
available for development and test qualification. There is no command allowlist
that treats limited smoke coverage as proof of a command's complete semantics.

## Replacement acceptance

1. Select a coherent command or implementation boundary and identify reusable
   upstream Rust source before writing another implementation.
2. Keep its exact source revision, dependency lock, license notices and failing
   upstream Git cases in the review.
3. Exercise the actual native Rit binary through Lustr. Cover accepted options,
   errors, exit status, output, repository effects, raw paths, SHA-1/SHA-256,
   configured helpers and relevant transport/platform behavior.
4. Compare with the pinned Git implementation. Keep deliberate behavioral
   differences explicit and preserve failing cases.
5. Change default routing only after the complete admitted contract passes.
   Select before mutation; do not fall back after a failure.
6. Retire connected implementation only when no remaining admitted operation
   requires it. Preserve corresponding source and notices for shipped versions.

Lustr coordinates compilation of Rust. Its presence does not mean C code is
automatically translated, arbitrary Rust is compiled by native Lust, or builds
are faster. Record the compiler route actually used and measure performance
with equal work before claiming an improvement.

## Remaining proof

Full upstream Git suite coverage, authenticated remote transports, all supported
platforms, helpers, installation layouts and release packaging remain required.
Historical selected native test results are useful evidence for their exact
revisions and cases. They are not certification of a finished native port.
