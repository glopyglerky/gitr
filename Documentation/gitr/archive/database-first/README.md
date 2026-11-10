# Deferred database and forge prototype

The owner has reset the delivery order: complete Git compatibility first, then
database and forge improvements. This directory preserves the former root
application and tests from `fe8e48a59e07c630b59f4a38c2c406b13f58c28b`.
It is not part of the active Cargo dependency graph. No repository database is
opened, converted, or deleted by the new frontend.

The archived manifest preserves its original paths and is evidence, not an
independently relocated build. Use the original Git commit to reproduce it.
The prior Grit and workflow donors remain in `vendor/` under their original
licenses for selective reuse. They are not linked into the active executable.

The selected future database remains redb, but database integration is deferred
until the Git compatibility gate is met. The archived tests remain useful for
that later integration; they do not describe the new executable's behavior.
