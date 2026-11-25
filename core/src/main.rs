use std::path::PathBuf;
use webforge_core::config::Config;

fn parse_arg(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "webforge_core=info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("serve-static") {
        let root = parse_arg(&args, "--root").unwrap_or_else(|| ".".to_string());
        let port: u16 = parse_arg(&args, "--port")
            .and_then(|v| v.parse().ok())
            .unwrap_or(8080);
        let host = parse_arg(&args, "--host").unwrap_or_else(|| "127.0.0.1".to_string());
        if let Err(e) = webforge_core::run_static_server(PathBuf::from(root), host, port).await {
            eprintln!("serve-static gagal: {}", e);
            std::process::exit(1);
        }
        return;
    }

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Konfigurasi gagal: {}", e);
            std::process::exit(1);
        }
    };
    if let Err(e) = webforge_core::run_management(config).await {
        eprintln!("webforge-core gagal: {}", e);
        std::process::exit(1);
    }
}
