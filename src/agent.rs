use anyhow::{Context, Result, bail};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct AgentKey {
    pub line: String,
    pub key_type: String,
    pub comment: String,
}

pub fn list_agent_keys() -> Result<Vec<AgentKey>> {
    let output = Command::new("ssh-add")
        .arg("-L")
        .output()
        .context("failed to run `ssh-add -L`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr.trim();
        if msg.is_empty() {
            bail!("`ssh-add -L` failed with status {}", output.status);
        }
        bail!("`ssh-add -L` failed: {msg}");
    }

    let stdout = String::from_utf8(output.stdout).context("ssh-add output was not valid UTF-8")?;
    let mut keys = Vec::new();

    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line.starts_with("The agent has no identities") {
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
