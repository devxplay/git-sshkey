use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct AgentKey {
    pub line: String,
    pub key_type: String,
    pub comment: String,
}

impl AgentKey {
    pub fn identifier(&self) -> String {
        if self.comment.is_empty() {
            let mut hasher = Sha256::new();
            hasher.update(self.line.as_bytes());
            hex::encode(hasher.finalize())[..8].to_string()
        } else {
            self.comment.clone()
        }
    }
}

pub fn list_agent_keys() -> Result<Vec<AgentKey>> {
    let output = Command::new(resolve_ssh_add_binary())
        .arg("-L")
        .output()
        .context("failed to run `ssh-add -L`")?;

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let stderr_str = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        let msg = stderr_str.trim();
        let out_msg = stdout_str.trim();

        if msg.contains("The agent has no identities")
            || out_msg.contains("The agent has no identities")
            || msg.contains("no identities")
            || out_msg.contains("no identities")
        {
            return Ok(Vec::new());
        }

        if msg.is_empty() {
            bail!("`ssh-add -L` failed with status {}", output.status);
        }
        bail!("`ssh-add -L` failed: {msg}");
    }

    let mut keys = Vec::new();

    for line in stdout_str
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line.starts_with("The agent has no identities") || line.contains("no identities") {
            return Ok(Vec::new());
        }

        let mut parts = line.split_whitespace();
        let key_type = parts.next().unwrap_or_default().to_string();
        let encoded_key = parts.next().unwrap_or_default().to_string();
        let comment = parts.collect::<Vec<_>>().join(" ");

        if key_type.is_empty() || encoded_key.is_empty() {
            continue;
        }

        keys.push(AgentKey {
            line: line.to_string(),
            key_type,
            comment,
        });
    }

    Ok(keys)
}

fn resolve_ssh_add_binary() -> PathBuf {
    if cfg!(windows) {
        for candidate in [
            "C:/Program Files/OpenSSH/ssh-add.exe",
            "C:/Windows/System32/OpenSSH/ssh-add.exe",
        ] {
            let path = PathBuf::from(candidate);
            if path.exists() {
                return path;
            }
        }
    }

    PathBuf::from("ssh-add")
}
