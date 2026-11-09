#!/usr/bin/env python3
"""Check the selected storage backend without building or changing repository data."""
import argparse
from pathlib import Path
import subprocess
import tomllib
import unittest

RETIRED = (
    "rust/gitr-store/", "rust/gitr-surreal-probe/",
    "rust/gitr/src/actions_remote.rs", "ci/gitr-store-", "ci/gitr-surreal-",
    ".github/workflows/gitr-store-qualification.yml",
    ".github/workflows/gitr-surreal-qualification.yml",
)


def dependency_names(value):
    if not isinstance(value, dict):
        return
    for key, item in value.items():
        if key in {"dependencies", "dev-dependencies", "build-dependencies"}:
            for alias, spec in item.items():
                yield alias
                if isinstance(spec, dict):
                    yield spec.get("package", alias)
        else:
            yield from dependency_names(item)


def validate_manifest(value):
    for name in dependency_names(value):
        if name.lower().replace("_", "-").startswith("surreal"):
            raise ValueError(f"retired database dependency: {name}")


def validate_lock(value):
    for package in value.get("package", []):
        name = package["name"]
        if name.lower().replace("_", "-").startswith("surreal"):
            raise ValueError(f"retired locked dependency: {name}")


def check(root):
    names = subprocess.check_output(
        ["git", "ls-files", "-z", "--", "rust", "ci", ".github/workflows"], cwd=root
    ).decode("utf-8").split("\0")
    for name in filter(None, names):
        if name.startswith(RETIRED):
            raise ValueError(f"retired migration path is active: {name}")
        path = root / name
        if name.startswith("rust/") and path.name in {"Cargo.toml", "Cargo.lock"}:
            value = tomllib.loads(path.read_text(encoding="utf-8"))
            (validate_manifest if path.name == "Cargo.toml" else validate_lock)(value)
    manifest = tomllib.loads((root / "rust/gitr/Cargo.toml").read_text(encoding="utf-8"))
    pin = manifest["dependencies"]["redb"]
    if isinstance(pin, dict):
        if pin.get("optional", False):
            raise ValueError("redb must remain a required dependency")
        pin = pin["version"]
    if not pin.startswith("="):
        raise ValueError("redb must retain an exact reviewed version pin")
    lock = tomllib.loads((root / "rust/gitr/Cargo.lock").read_text(encoding="utf-8"))
    versions = [p["version"] for p in lock["package"] if p["name"] == "redb"]
    if versions != [pin[1:]]:
        raise ValueError("redb manifest and lockfile pin disagree")
    print(f"storage policy: redb {versions[0]}; no active SurrealDB dependencies or migration paths")


class PolicyTests(unittest.TestCase):
    def test_redb_is_allowed(self):
        validate_manifest(tomllib.loads('[dependencies]\nredb = "=4.5.0"'))
        validate_lock({"package": [{"name": "redb", "version": "4.5.0"}]})

    def test_direct_dependency_is_rejected(self):
        with self.assertRaises(ValueError):
            validate_manifest({"dependencies": {"surrealdb": "4"}})

    def test_renamed_optional_target_dependency_is_rejected(self):
        with self.assertRaises(ValueError):
            validate_manifest({"target": {"cfg(unix)": {"dependencies": {
                "db": {"package": "surrealkv", "optional": True, "version": "1"}
            }}}})

    def test_workspace_and_development_dependencies_are_rejected(self):
        for section in ["dependencies", "dev-dependencies", "build-dependencies"]:
            with self.subTest(section=section), self.assertRaises(ValueError):
                validate_manifest({"workspace": {section: {"surrealdb-core": "4"}}})

    def test_transitive_locked_dependency_is_rejected(self):
        with self.assertRaises(ValueError):
            validate_lock({"package": [{"name": "surrealdb-core", "version": "4.0.0"}]})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        unittest.main(argv=[__file__])
    else:
        check(Path(__file__).resolve().parents[1])
