use crate::util::{repo_root, run};
use anyhow::{bail, Result};
use std::path::PathBuf;
use std::process::Command;

fn wrapper_script() -> Result<PathBuf> {
    let script = repo_root()?.join("scripts").join("run-in-devcontainer.sh");
    if !script.exists() {
        bail!(
            "devcontainer wrapper requires scripts/run-in-devcontainer.sh — not found at {}",
            script.display()
        );
    }
    Ok(script)
}

pub(crate) fn run_in_devcontainer(args: &[&str]) -> Result<()> {
    let script = wrapper_script()?;
    let mut command = Command::new("bash");
    command.arg(script);
    command.args(args);
    run(command)
}
