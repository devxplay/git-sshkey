use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;

fn run_git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn get_local_ssh_command() -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["config", "--local", "--get", "core.sshCommand"])
        .output()
        .context("failed to read local core.sshCommand")?;

    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }

    Ok(None)
}

pub fn get_global_ssh_command() -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["config", "--global", "--get", "core.sshCommand"])
        .output()
        .context("failed to read global core.sshCommand")?;

    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }

    Ok(None)
}

pub fn inherited_ssh_binary() -> Result<String> {
    if let Some(global) = get_global_ssh_command()? {
        let candidate = first_token(&global);
        if !candidate.is_empty() {
            return Ok(candidate);
        }
    }

    if cfg!(windows) {
        Ok("C:/Windows/System32/OpenSSH/ssh.exe".to_string())
    } else {
        Ok("ssh".to_string())
    }
}

pub fn set_local_ssh_command(command: &str) -> Result<()> {
    run_git(&["config", "--local", "core.sshCommand", command])?;
    Ok(())
}

pub fn unset_local_ssh_command() -> Result<()> {
    let output = Command::new("git")
        .args(["config", "--local", "--unset", "core.sshCommand"])
        .output()
        .context("failed to clear local core.sshCommand")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("No such section or key") {
        return Ok(());
    }

    bail!("failed to clear local core.sshCommand: {}", stderr.trim());
}

pub fn ensure_pub_storage_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not locate home directory")?;
    let dir = home.join(".ssh").join("git-sshkey");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create directory `{}`", dir.display()))?;
    Ok(dir)
}

fn first_token(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Some(stripped) = trimmed.strip_prefix('"') {
        if let Some(end) = stripped.find('"') {
            return stripped[..end].to_string();
        }
    }

    trimmed
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}
