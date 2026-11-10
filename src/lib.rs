// SPDX-License-Identifier: MIT OR Apache-2.0
//! Git-compatible entrypoints over the imported native Rust engine.
//! Repository operations have one implementation; no C Git fallback or database
//! overlay is selected by command name.

use std::{ffi::OsString, path::Path, process::ExitCode};

/// Run the standard Git interface, including installed `git-*` helper names.
pub fn main() -> ExitCode {
    let mut invocation = std::env::args_os();
    let executable = invocation.next().unwrap_or_else(|| OsString::from("rit"));
    let basename = Path::new(&executable)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("rit");
    let mut args: Vec<String> = invocation.map(sley_cli::argv_string_from_os).collect();
    let result = if basename == "scalar" {
        sley_cli::run_scalar(args)
    } else {
        if let Some(helper) = basename.strip_prefix("git-") {
            args.insert(0, helper.to_owned());
        }
        sley_cli::run(args)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if sley_cli::cli_reported_status(&error).is_none() {
                match &error {
                    sley::GitError::InvalidFormat(message)
                        if message.starts_with("fatal: ") || message.starts_with("error: ") =>
                    {
                        eprintln!("{message}");
                    }
                    _ => eprintln!("{basename}: {error}"),
                }
            }
            ExitCode::from(sley_cli::cli_exit_code(&error) as u8)
        }
    }
}
