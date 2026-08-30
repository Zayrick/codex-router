use std::path::PathBuf;

use anyhow::{Result, bail};

#[tokio::main]
async fn main() -> Result<()> {
    let Some(config_path) = config_path()? else {
        return Ok(());
    };
    codex_router::server::run(config_path).await
}

fn config_path() -> Result<Option<PathBuf>> {
    let mut arguments = std::env::args_os();
    let program = arguments
        .next()
        .and_then(|value| PathBuf::from(value).file_name().map(|name| name.to_owned()))
        .and_then(|value| value.to_str().map(str::to_owned))
        .unwrap_or_else(|| "codex-router".into());
    let mut path = None;
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("-c" | "--config") => {
                let Some(value) = arguments.next() else {
                    bail!("--config requires a file path");
                };
                if path.replace(PathBuf::from(value)).is_some() {
                    bail!("--config may only be specified once");
                }
            }
            Some("-h" | "--help") => {
                println!("Usage: {program} [--config <path>]\n\nDefaults to ./config.toml");
                return Ok(None);
            }
            Some("-V" | "--version") => {
                println!("{program} {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            Some(value) => bail!("unknown argument: {value}"),
            None => bail!("arguments must be valid UTF-8"),
        }
    }
    Ok(Some(path.unwrap_or_else(|| PathBuf::from("config.toml"))))
}
