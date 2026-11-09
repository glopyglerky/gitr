use std::io::{self, Write};
use std::time::Duration;

fn main() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let advertisement = std::fs::read(executable.with_extension("advertisement"))?;
    let report = std::fs::read(executable.with_extension("report"))?;
    let mut stdout = io::stdout().lock();
    stdout.write_all(&advertisement)?;
    stdout.flush()?;
    std::thread::spawn(|| {
        io::copy(&mut io::stdin().lock(), &mut io::sink()).expect("drain push request");
    });
    stdout.write_all(&report)?;
    stdout.flush()?;
    // Preserve the fixture's open input pipe while the client writes its pack.
    // The client waits for stdout EOF before closing its own output pipe.
    std::thread::sleep(Duration::from_secs(1));
    Ok(())
}
