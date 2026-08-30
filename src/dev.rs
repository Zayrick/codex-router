use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use notify::{Config, Event, PollWatcher, RecursiveMode, Watcher};
use tokio::{
    process::{Child, Command},
    sync::mpsc,
    time,
};

const DEV_CHILD_ENV: &str = "CODEX_ROUTER_DEV_CHILD";
const DISABLE_DEV_ENV: &str = "CODEX_ROUTER_NO_DEV";
const VITE_HOST: &str = "127.0.0.1:5173";

pub fn should_supervise() -> bool {
    std::env::var_os(DEV_CHILD_ENV).is_none() && std::env::var_os(DISABLE_DEV_ENV).is_none()
}

pub async fn run() -> Result<()> {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let frontend = project.join("frontend");
    install_frontend_dependencies(&frontend).await?;

    let mut vite = spawn_vite(&frontend)?;
    wait_for_vite(&mut vite).await?;
    let mut backend = Some(spawn_backend()?);
    let (watcher, mut changes) = rust_watcher(&project)?;
    let mut process_check = time::interval(Duration::from_millis(500));
    process_check.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    eprintln!("[dev] React HMR is ready at http://{VITE_HOST}");
    eprintln!("[dev] watching Rust sources; successful builds restart the server");

    loop {
        tokio::select! {
            _ = shutdown_signal() => break,
            event = changes.recv() => {
                let Some(event) = event else {
                    bail!("Rust source watcher stopped unexpectedly");
                };
                if let Err(error) = event {
                    eprintln!("[dev] source watcher error: {error}");
                    continue;
                }
                time::sleep(Duration::from_millis(150)).await;
                while changes.try_recv().is_ok() {}
                eprintln!("[dev] rebuilding Rust server...");
                if build_backend(&project).await? {
                    if let Some(mut child) = backend.take() {
                        stop(&mut child).await;
                    }
                    backend = Some(spawn_backend()?);
                    eprintln!("[dev] Rust server restarted");
                } else {
                    eprintln!("[dev] Rust build failed; the previous server remains active");
                }
            }
            _ = process_check.tick() => {
                if let Some(status) = vite.try_wait().context("failed to inspect the Vite process")? {
                    bail!("Vite exited unexpectedly with {status}");
                }
                if let Some(child) = backend.as_mut()
                    && let Some(status) = child.try_wait().context("failed to inspect the Rust server process")?
                {
                    eprintln!("[dev] Rust server exited with {status}; it will restart after the next successful Rust build");
                    backend = None;
                }
            }
        }
    }

    drop(watcher);
    if let Some(mut child) = backend {
        stop(&mut child).await;
    }
    stop(&mut vite).await;
    Ok(())
}

async fn install_frontend_dependencies(frontend: &Path) -> Result<()> {
    let status = Command::new("pnpm")
        .args(["install", "--frozen-lockfile", "--prefer-offline"])
        .current_dir(frontend)
        .status()
        .await
        .context("failed to start pnpm; install pnpm 11 and Node.js 22.12 or newer")?;
    if !status.success() {
        bail!("pnpm install failed with {status}");
    }
    Ok(())
}

fn spawn_vite(frontend: &Path) -> Result<Child> {
    let vite = frontend.join("node_modules/vite/bin/vite.js");
    Command::new("node")
        .arg(vite)
        .current_dir(frontend)
        .kill_on_drop(true)
        .spawn()
        .context("failed to start the Vite development server")
}

async fn wait_for_vite(vite: &mut Child) -> Result<()> {
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(VITE_HOST).await.is_ok() {
            return Ok(());
        }
        if let Some(status) = vite
            .try_wait()
            .context("failed to inspect the Vite process")?
        {
            bail!("Vite exited before becoming ready with {status}");
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    bail!("Vite did not become ready at http://{VITE_HOST}")
}

fn spawn_backend() -> Result<Child> {
    let executable = std::env::current_exe().context("failed to locate the development binary")?;
    Command::new(executable)
        .args(std::env::args_os().skip(1))
        .env(DEV_CHILD_ENV, "1")
        .kill_on_drop(true)
        .spawn()
        .context("failed to start the Rust server")
}

async fn build_backend(project: &Path) -> Result<bool> {
    let status = Command::new(env!("CARGO"))
        .args(["build", "--bin", env!("CARGO_PKG_NAME")])
        .current_dir(project)
        .status()
        .await
        .context("failed to start Cargo for a development rebuild")?;
    Ok(status.success())
}

fn rust_watcher(
    project: &Path,
) -> Result<(PollWatcher, mpsc::UnboundedReceiver<notify::Result<Event>>)> {
    let project = project.to_path_buf();
    let source = project.join("src");
    let (sender, receiver) = mpsc::unbounded_channel();
    let filter_root = project.clone();
    let mut watcher = PollWatcher::new(
        move |result: notify::Result<Event>| match result {
            Ok(event)
                if event
                    .paths
                    .iter()
                    .any(|path| rust_build_input(path, &filter_root)) =>
            {
                let _ = sender.send(Ok(event));
            }
            Ok(_) => {}
            Err(error) => {
                let _ = sender.send(Err(error));
            }
        },
        Config::default()
            .with_poll_interval(Duration::from_millis(300))
            .with_compare_contents(true),
    )
    .context("failed to create the Rust source watcher")?;
    watcher
        .watch(&source, RecursiveMode::Recursive)
        .with_context(|| format!("failed to watch {}", source.display()))?;
    watcher
        .watch(&project, RecursiveMode::NonRecursive)
        .with_context(|| format!("failed to watch {}", project.display()))?;
    Ok((watcher, receiver))
}

fn rust_build_input(path: &Path, project: &Path) -> bool {
    let source = project.join("src");
    if path.starts_with(source) {
        return path.extension() == Some(OsStr::new("rs"));
    }
    ["Cargo.toml", "Cargo.lock", "build.rs"]
        .iter()
        .any(|name| path == project.join(name))
}

async fn stop(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill().await;
    }
    let _ = child.wait().await;
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watches_only_rust_build_inputs() {
        let root = Path::new("/project");
        assert!(rust_build_input(
            Path::new("/project/src/server/mod.rs"),
            root
        ));
        assert!(rust_build_input(Path::new("/project/Cargo.toml"), root));
        assert!(rust_build_input(Path::new("/project/build.rs"), root));
        assert!(!rust_build_input(
            Path::new("/project/frontend/src/main.tsx"),
            root
        ));
        assert!(!rust_build_input(
            Path::new("/project/target/debug/app"),
            root
        ));
    }
}
