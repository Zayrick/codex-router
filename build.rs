use std::{env, path::Path, process::Command};

fn main() {
    for input in [
        "frontend/index.html",
        "frontend/package.json",
        "frontend/pnpm-lock.yaml",
        "frontend/pnpm-workspace.yaml",
        "frontend/src",
        "frontend/tsconfig.app.json",
        "frontend/tsconfig.json",
        "frontend/tsconfig.node.json",
        "frontend/vite.config.ts",
    ] {
        println!("cargo::rerun-if-changed={input}");
    }

    let output = env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR");
    let assets = Path::new(&output).join("frontend");
    println!(
        "cargo::rustc-env=CODEX_ROUTER_FRONTEND_DIST={}",
        assets.display()
    );

    if env::var("DEBUG").as_deref() == Ok("true") {
        return;
    }

    let frontend = Path::new("frontend");
    run(
        Command::new("pnpm")
            .args(["install", "--frozen-lockfile", "--prefer-offline"])
            .current_dir(frontend),
        "install the frontend dependencies",
    );
    run(
        Command::new("pnpm")
            .args([
                "exec",
                "tsc",
                "-b",
                "tsconfig.app.json",
                "tsconfig.node.json",
            ])
            .current_dir(frontend),
        "type-check the frontend",
    );
    run(
        Command::new("pnpm")
            .args(["exec", "vite", "build", "--outDir"])
            .arg(&assets)
            .arg("--emptyOutDir")
            .current_dir(frontend),
        "build the embedded frontend",
    );
}

fn run(command: &mut Command, action: &str) {
    match command.status() {
        Ok(status) if status.success() => {}
        Ok(status) => panic!("failed to {action}: process exited with {status}"),
        Err(error) => panic!("failed to {action}: {error}"),
    }
}
