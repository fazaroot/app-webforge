use std::io::Read;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut stdin = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut stdin) {
        eprintln!("webforge-sendmail: gagal membaca stdin: {}", e);
        return ExitCode::from(1);
    }
    match webforge_sendmail_shim::run(&args, &stdin) {
        Ok(message) => {
            println!("{}", message);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("webforge-sendmail: {}", e);
            ExitCode::from(1)
        }
    }
}
