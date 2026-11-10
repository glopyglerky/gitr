use std::fs;
use std::io::{self, Write};

fn main() -> io::Result<()> {
    io::copy(&mut io::stdin().lock(), &mut io::sink())?;
    let executable = std::env::current_exe()?;
    let record = executable.with_extension("record");
    if !record.is_file() {
        eprintln!("policy: pushes are blocked");
        std::process::exit(1);
    }
    let count: usize = std::env::var("GIT_PUSH_OPTION_COUNT")
        .unwrap_or_else(|_| "0".to_owned()).parse().expect("push option count");
    let mut file = fs::File::create(fs::read_to_string(record)?)?;
    writeln!(file, "count={count}")?;
    for i in 0..count {
        let value = std::env::var(format!("GIT_PUSH_OPTION_{i}")).unwrap_or_default();
        writeln!(file, "{i}={value}")?;
    }
    Ok(())
}
