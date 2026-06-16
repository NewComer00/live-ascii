use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::process::Command;

const PURISM_REPO: &str = "https://github.com/SakuraMotion/PurismCore.git";
const PURISM_TAG: &str = "v1.0.1";

fn run_command(program: &str, args: &[&str], cwd: &PathBuf) -> Result<(), Box<dyn Error>> {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .map_err(|e| format!(
            "Failed to execute '{}'. Is it installed and on PATH?\n  Caused by: {}",
            program, e
        ))?;

    if !status.success() {
        return Err(format!(
            "'{} {}' failed with exit code {:?}",
            program,
            args.join(" "),
            status.code()
        ).into());
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let purism_dir = manifest_dir.join("vendor").join("purismcore");
    let build_dir = purism_dir.join("build");
    let lib_file = build_dir.join("libPurismCore.a");

    let target_os = env::var("CARGO_CFG_TARGET_OS")?;
    let os_arg = match target_os.as_str() {
        "windows" => "windows",
        "macos" => "macos",
        _ => "linux",
    };

    if !purism_dir.join(".git").exists() {
        println!("cargo:warning=Cloning PurismCore {} from GitHub...", PURISM_TAG);
        run_command("git", &[
            "clone",
            "--depth", "1",
            "--branch", PURISM_TAG,
            PURISM_REPO,
            purism_dir.to_str().ok_or("vendor path is not valid UTF-8")?,
        ], &manifest_dir)?;
    }

    if !lib_file.exists() {
        println!("cargo:warning=Building PurismCore (this may take a moment)...");
        run_command("make", &["static-lib", "ABI=v6", &format!("OS={}", os_arg)], &purism_dir)?;
    }

    println!("cargo:rustc-link-search=native={}", build_dir.display());
    println!("cargo:rustc-link-lib=static=PurismCore");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", lib_file.display());

    Ok(())
}
