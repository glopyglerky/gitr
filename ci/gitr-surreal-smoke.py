#!/usr/bin/env python3
"""Run the isolated Rust probe with a fresh datastore; never uses repo data."""
from pathlib import Path
import selectors
import subprocess
import tempfile

binary = Path("rust/gitr-surreal-probe/target/debug/gitr-surreal-probe").resolve()
with tempfile.TemporaryDirectory(prefix="gitr-surreal-proof-") as directory:
    path = str(Path(directory) / "database")
    def run(phase):
        result = subprocess.run([str(binary), phase, path], capture_output=True, text=True, timeout=90)
        if result.returncode:
            raise AssertionError(f"{phase}: {result.stderr}\n{result.stdout}")
        print(result.stdout, end="")
    run("seed")
    run("verify")
    with tempfile.TemporaryFile(mode="w+") as errors:
        child = subprocess.Popen([str(binary), "ack-and-wait", path], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, text=True)
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                if not selector.select(60):
                    raise AssertionError("writer did not acknowledge within timeout")
                if child.stdout.readline().strip() != "ACK":
                    errors.seek(0)
                    raise AssertionError("writer failed before acknowledgement: " + errors.read())
            run("locked")
            child.kill()
            child.wait(timeout=10)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=10)
            child.stdin.close()
            child.stdout.close()
    run("recover")
print("SurrealKV single-node qualification passed; SurrealDS/HA and Gitr migration are NOT tested.")
