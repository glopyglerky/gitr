// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reference-selector output is compared with the pinned Git oracle.

use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};

struct Repository(PathBuf);

impl Repository {
    fn new(format: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sley-rev-parse-pseudo-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("empty-path")).unwrap();
        let repo = Self(root);
        repo.git(&[
            "init",
            "-q",
            "-b",
            "main",
            &format!("--object-format={format}"),
        ]);
        repo.git(&["commit", "--allow-empty", "-q", "-m", "first"]);
        repo.git(&["branch", "topic"]);
        repo.git(&["tag", "topic"]);
        repo.git(&["commit", "--allow-empty", "-q", "-m", "second"]);
        repo.git(&["tag", "-a", "v1", "-m", "annotation"]);
        for name in [
            "refs/heads/team/one",
            "refs/heads/team/two",
            "refs/remotes/origin/main",
            "refs/remotes/origin/topic",
            "refs/custom/checkpoint",
        ] {
            repo.git(&["update-ref", name, "HEAD"]);
        }
        repo
    }

    fn output(&self, program: impl AsRef<OsStr>, args: &[&str], empty_path: bool) -> Output {
        self.output_with_env(program, args, empty_path, &[])
    }

    fn output_with_env(
        &self,
        program: impl AsRef<OsStr>,
        args: &[&str],
        empty_path: bool,
        environment: &[(&str, &OsStr)],
    ) -> Output {
        let mut command = sley_testkit::hermetic_git_command_with_identity(program);
        command
            .current_dir(&self.0)
            .args(args)
            .envs(environment.iter().copied());
        if empty_path {
            command.env("PATH", self.0.join("empty-path"));
        }
        command.output().unwrap()
    }

    fn git(&self, args: &[&str]) -> Output {
        let output = self.output(sley_testkit::oracle_git(), args, false);
        assert!(
            output.status.success(),
            "oracle {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn assert_matches(&self, args: &[&str]) {
        self.assert_matches_with_env(args, &[]);
    }

    fn assert_matches_with_env(&self, args: &[&str], environment: &[(&str, &OsStr)]) {
        let expected = self.output_with_env(sley_testkit::oracle_git(), args, false, environment);
        assert!(
            expected.status.success(),
            "oracle {args:?}: {}",
            String::from_utf8_lossy(&expected.stderr)
        );
        let binary = std::env::var_os("GITR_TEST_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(sley_testkit::sley_bin!()));
        assert!(
            binary.is_absolute(),
            "GITR_TEST_BIN must be an absolute path"
        );
        let actual = self.output_with_env(binary, args, true, environment);
        assert_eq!(
            actual.status.code(),
            expected.status.code(),
            "{args:?}: {}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert!(
            actual.stdout == expected.stdout,
            "stdout for {args:?}\nexpected: {}\nactual: {}",
            String::from_utf8_lossy(&expected.stdout),
            String::from_utf8_lossy(&actual.stdout)
        );
        assert!(
            actual.stderr == expected.stderr,
            "stderr for {args:?}\nexpected: {}\nactual: {}",
            String::from_utf8_lossy(&expected.stderr),
            String::from_utf8_lossy(&actual.stderr)
        );
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn symbolic_reference_selectors_match_git_with_no_git_on_path() {
    for format in ["sha1", "sha256"] {
        let repo = Repository::new(format);
        for selector in [
            "--all",
            "--branches",
            "--branches=team",
            "--tags",
            "--tags=v*",
            "--remotes",
            "--remotes=origin",
            "--glob=refs/heads/team/*",
        ] {
            for mode in ["--symbolic", "--symbolic-full-name"] {
                repo.assert_matches(&["rev-parse", "--revs-only", mode, selector]);
            }
            repo.assert_matches(&["rev-parse", selector]);
        }
    }
}

#[test]
fn abbreviated_reference_selectors_honor_ambiguity_modes_and_configuration() {
    for format in ["sha1", "sha256"] {
        let repo = Repository::new(format);
        for config in [None, Some("false"), Some("true")] {
            for mode in ["--abbrev-ref", "--abbrev-ref=strict", "--abbrev-ref=loose"] {
                for selector in [
                    "--all",
                    "--branches",
                    "--tags",
                    "--remotes",
                    "--glob=refs/*",
                    "--glob=refs/heads/team/*",
                ] {
                    let setting = config.map(|value| format!("core.warnAmbiguousRefs={value}"));
                    let mut args = Vec::new();
                    if let Some(setting) = &setting {
                        args.extend(["-c", setting.as_str()]);
                    }
                    args.extend(["rev-parse", mode, selector]);
                    repo.assert_matches(&args);
                }
            }
        }
    }
}

#[test]
fn reference_selectors_resolve_symbolic_aliases_and_skip_dangling_or_cyclic_refs() {
    for format in ["sha1", "sha256"] {
        let repo = Repository::new(format);
        repo.git(&["branch", "missing"]);
        repo.git(&["branch", "cycle"]);
        for (name, target) in [
            ("refs/remotes/origin/HEAD", "refs/remotes/origin/main"),
            ("refs/remotes/origin/default", "refs/remotes/origin/HEAD"),
            ("refs/remotes/missing/HEAD", "refs/heads/absent"),
            ("refs/remotes/cycle/a", "refs/remotes/cycle/b"),
            ("refs/remotes/cycle/b", "refs/remotes/cycle/a"),
            ("refs/remotes/cycle/HEAD", "refs/remotes/cycle/a"),
        ] {
            repo.git(&["symbolic-ref", name, target]);
        }
        for selector in ["--all", "--branches", "--remotes", "--glob=refs/remotes/*"] {
            repo.assert_matches(&["rev-parse", selector]);
            for mode in ["--symbolic", "--symbolic-full-name", "--abbrev-ref"] {
                repo.assert_matches(&["rev-parse", mode, selector]);
            }
        }
    }
}

#[test]
fn reference_selector_modes_and_exclusions_follow_argument_order() {
    for format in ["sha1", "sha256"] {
        let repo = Repository::new(format);
        for args in [
            vec![
                "rev-parse",
                "--symbolic-full-name",
                "--all",
                "--symbolic",
                "--branches",
            ],
            vec![
                "rev-parse",
                "--symbolic",
                "--branches",
                "--symbolic-full-name",
                "--all",
            ],
            vec![
                "rev-parse",
                "--abbrev-ref=loose",
                "--all",
                "--abbrev-ref=strict",
                "--all",
                "--abbrev-ref",
                "--all",
            ],
            vec!["rev-parse", "--symbolic", "--abbrev-ref", "--all"],
            vec!["rev-parse", "--abbrev-ref", "--symbolic", "--all"],
            vec!["rev-parse", "HEAD", "--symbolic-full-name", "--all", "HEAD"],
            vec![
                "rev-parse",
                "--symbolic-full-name",
                "--exclude=refs/tags/*",
                "--all",
                "--all",
            ],
            vec![
                "rev-parse",
                "--symbolic",
                "--exclude=team/*",
                "--branches",
                "--branches",
            ],
            vec![
                "rev-parse",
                "--abbrev-ref",
                "--exclude=refs/remotes/*",
                "--glob=refs/*",
                "--remotes",
            ],
            vec![
                "-c",
                "transfer.hideRefs=refs/heads/team",
                "rev-parse",
                "--symbolic-full-name",
                "--exclude-hidden=fetch",
                "--all",
                "--all",
            ],
        ] {
            repo.assert_matches(&args);
        }
    }
}

#[test]
fn reference_selectors_cover_root_refs_global_config_and_broken_aliases() {
    for format in ["sha1", "sha256"] {
        {
            let repo = Repository::new(format);
            let config = repo.0.join("global-config");
            fs::write(&config, "[core]\nwarnAmbiguousRefs = false\n").unwrap();
            for mode in [
                "--symbolic-full-name",
                "--abbrev-ref",
                "--abbrev-ref=strict",
                "--abbrev-ref=loose",
            ] {
                for selector in ["--all", "--branches", "--tags"] {
                    repo.assert_matches_with_env(
                        &["rev-parse", mode, selector],
                        &[("GIT_CONFIG_GLOBAL", config.as_os_str())],
                    );
                }
            }
        }
        {
            let repo = Repository::new(format);
            let oid = repo.git(&["rev-parse", "HEAD"]).stdout;
            for name in ["FETCH_HEAD", "MERGE_HEAD", "ORIG_HEAD", "root-topic"] {
                repo.git(&["branch", name]);
                fs::write(repo.0.join(".git").join(name), &oid).unwrap();
            }
            for mode in ["--symbolic-full-name", "--abbrev-ref"] {
                for selector in ["--branches", "--all"] {
                    repo.assert_matches(&["rev-parse", mode, selector]);
                }
            }
        }
        {
            let repo = Repository::new(format);
            repo.git(&[
                "symbolic-ref",
                "refs/remotes/topic/HEAD",
                "refs/heads/absent",
            ]);
            for mode in ["--symbolic-full-name", "--abbrev-ref"] {
                for selector in ["--branches", "--tags"] {
                    repo.assert_matches(&["rev-parse", mode, selector]);
                }
            }
        }
        {
            let repo = Repository::new(format);
            fs::write(repo.0.join(".git/BROKEN_REF"), "not an object id\n").unwrap();
            repo.git(&["symbolic-ref", "refs/remotes/origin/broken", "BROKEN_REF"]);
            for selector in ["--all", "--remotes", "--glob=refs/remotes/*"] {
                repo.assert_matches(&["rev-parse", selector]);
                for mode in ["--symbolic", "--symbolic-full-name", "--abbrev-ref"] {
                    repo.assert_matches(&["rev-parse", mode, selector]);
                }
            }
        }
    }
}
