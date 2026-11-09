#!/usr/bin/env python3
"""End-to-end proof: actual compiled Rust/Wasm reads committed, not dirty bytes."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

binary = (Path(os.environ.get("CARGO_TARGET_DIR", "target")) / ("debug/gitr" + (".exe" if os.name == "nt" else ""))).resolve()
wasm = (Path(os.environ.get("GITR_EXAMPLE_TARGET_DIR", "crates/action-example/target")) / "wasm32-unknown-unknown/release/gitr_action_example.wasm").resolve()

def run(args, cwd, data=None, ok=True):
    result = subprocess.run(args, cwd=cwd, input=data, capture_output=True, check=False, timeout=30)
    if ok and result.returncode != 0:
        raise AssertionError(result.stderr.decode(errors="replace"))
    return result

with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    run(["git", "init", "-q"], root)
    run(["git", "config", "user.name", "Gitr qualification"], root)
    run(["git", "config", "user.email", "gitr@example.invalid"], root)
    run(["git", "config", "core.autocrlf", "false"], root)
    (root / "source.rs").write_bytes(b"fn main() {}\n")
    run(["git", "add", "source.rs"], root)
    run(["git", "commit", "-qm", "immutable input"], root)
    module = json.loads(run([str(binary), "actions", "install", str(wasm)], root).stdout)["module_sha256"]
    (root / "source.rs").write_bytes(b"dirty bytes with trailing whitespace \n")
    submitted = json.loads(run([str(binary), "actions", "submit", "source-policy", module, "--path", "source.rs"], root).stdout)
    result = json.loads(run([str(binary), "actions", "run", "source-policy"], root).stdout)
    assert result["status"] == "passed" and result["attempt"] == 1
    artifact = json.loads(run([str(binary), "actions", "artifact", "source-policy"], root).stdout)
    assert artifact == {"bytes": 13, "lines": 1, "trailing_whitespace": False}, artifact
    control = json.dumps({"operation": "status", "job": "source-policy"}).encode()
    assert json.loads(run([str(binary), "actions", "control"], root, control).stdout) == result
    assert json.loads(run([str(binary), "actions", "run", "source-policy"], root).stdout) == result
    run(["git", "add", "source.rs"], root)
    run(["git", "commit", "-qm", "bad policy input"], root)
    run([str(binary), "actions", "submit", "bad-policy", module, "--path", "source.rs"], root)
    failed = run([str(binary), "actions", "run", "bad-policy"], root, ok=False)
    assert failed.returncode != 0 and json.loads(failed.stdout)["status"] == "failed"
    assert run([str(binary), "actions", "artifact", "bad-policy"], root, ok=False).returncode != 0
    run(["git", "fsck", "--strict"], root)
    print(json.dumps({"rust_wasm_git_snapshot_smoke": "passed", "module_sha256": module,
        "commit_oid": submitted["spec"]["commit_oid"], "artifact": artifact,
        "failed_policy_exit": failed.returncode, "receipt_retry": "identical", "dirty_worktree": "excluded"}))
