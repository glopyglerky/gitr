use std::fs::{self, OpenOptions};
use std::io::{self, Write};

fn main() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    let action = args.last().map(String::as_str).unwrap_or("");
    io::copy(&mut io::stdin().lock(), &mut io::sink())?;
    if executable.with_extension("record").is_file() {
        let record = fs::read_to_string(executable.with_extension("record"))?;
        let mut file = OpenOptions::new().create(true).append(true).open(record)?;
        if executable.with_extension("record-arguments").is_file() {
            writeln!(file, "action={action} arg={}", args.first().map(String::as_str).unwrap_or(""))?;
        } else {
            writeln!(file, "{action}")?;
        }
    }
    let status: i32 = fs::read_to_string(executable.with_extension("exit-code"))?
        .parse().expect("fixture exit code");
    if status != 0 {
        std::process::exit(status);
    }
    if action == "get" {
        io::stdout().lock().write_all(&fs::read(executable.with_extension("response"))?)?;
    }
    Ok(())
}
