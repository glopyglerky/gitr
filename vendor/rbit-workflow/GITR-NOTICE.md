# Rbit workflow import

Complete `crates/rbit-workflow` from GitRbit/rbit at
`39f932a00b7aa35b324e06a9ff2343a57d4bed76`, including its tests and fixtures.
Original code remains MIT OR Apache-2.0; Gitr uses the offered MIT grant.
Both upstream license texts and the unmodified original crate manifest are retained.
The generated provenance manifest records every original file's SHA-256.

Local changes resolve workspace inheritance and make an overflowing deadline
expire instead of disabling the deadline. The latter includes a regression test.
No Rbit server, SQLite backend, runner subprocess, or secret materialization is
imported into Gitr's execution service. Keep future updates pinned and review the
local patch and full conformance corpus rather than replacing notices or tests.

Rust-only audit: this import contains Rust implementation/tests and passive
YAML/JSON parser fixtures. Their `run` fields are never executed by this crate
or Gitr. No donor files were changed or removed in the 2026-11-09 cleanup;
original hashes and the existing local divergence above remain applicable.
