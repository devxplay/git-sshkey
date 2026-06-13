use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
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

pub fn get_global_default_key() -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["config", "--global", "--get", "git-sshkey.default"])
        .output()
        .context("failed to read global git-sshkey.default")?;

    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }

    Ok(None)
}

pub fn set_global_default_key(identifier: &str) -> Result<()> {
    run_git(&["config", "--global", "git-sshkey.default", identifier])?;
    Ok(())
}

pub fn get_remote_url(remote_name: Option<&str>) -> Result<Option<String>> {
    let remote = match remote_name {
        Some(name) => name.to_string(),
        None => {
            // First try 'origin' directly — if it exists, return its URL immediately
            // to avoid a redundant second subprocess call.
            let origin_output = Command::new("git")
                .args(["remote", "get-url", "origin"])
                .output()
                .context("failed to check remote origin")?;
            if origin_output.status.success() {
                let url = String::from_utf8_lossy(&origin_output.stdout)
                    .trim()
                    .to_string();
                if !url.is_empty() {
                    return Ok(Some(url));
                }
            }

            // If 'origin' does not exist, list all remotes and use the first one
            let remotes_output = Command::new("git")
                .args(["remote"])
                .output()
                .context("failed to list remotes")?;
            if remotes_output.status.success() {
                let stdout = String::from_utf8_lossy(&remotes_output.stdout);
                if let Some(r) = stdout.lines().map(str::trim).find(|l| !l.is_empty()) {
                    r.to_string()
                } else {
                    return Ok(None);
                }
            } else {
                return Ok(None);
            }
        }
    };

    // Resolve the remote name to its URL
    let output = Command::new("git")
        .args(["remote", "get-url", &remote])
        .output();

    if let Ok(output) = output
        && output.status.success()
    {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }

    Ok(None)
}

fn cache_path_for_remote(remote: &str) -> Result<PathBuf> {
    let normalized = normalize_remote_url(remote);
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let hashed = hex::encode(hasher.finalize());
    Ok(ensure_pub_storage_dir()?.join("cache").join(hashed))
}

pub fn get_cached_key_for_remote(remote: &str) -> Result<Option<String>> {
    let path = cache_path_for_remote(remote)?;
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        return Ok(Some(content.trim().to_string()));
    }
    Ok(None)
}

pub fn set_cached_key_for_remote(remote: &str, public_key_line: &str) -> Result<()> {
    let path = cache_path_for_remote(remote)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, public_key_line.trim())?;
    Ok(())
}

pub fn clear_cached_key_for_remote(remote: &str) -> Result<()> {
    let path = cache_path_for_remote(remote)?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn normalize_remote_url(url: &str) -> String {
    let mut s = url.trim().to_lowercase();

    // Strip protocol scheme (e.g. ssh://, https://)
    if let Some(idx) = s.find("://") {
        s = s[idx + 3..].to_string();
    }

    // Strip user prefix if any (e.g. git@)
    if let Some(idx) = s.find('@') {
        let has_slash_before_at = s
            .find('/')
            .map(|slash_idx| slash_idx < idx)
            .unwrap_or(false);
        if !has_slash_before_at {
            s = s[idx + 1..].to_string();
        }
    }

    // Strip standard port numbers (e.g. :22/ for SSH, :443/ for HTTPS) so that
    // ssh://git@github.com:22/foo/bar and git@github.com:foo/bar share the same cache key.
    // Only strip when the port is followed by '/' (host:port/path), not scp-style (host:path).
    for port in ["22", "443", "9418"] {
        let pattern = format!(":{port}/");
        if let Some(idx) = s.find(&pattern) {
            // Verify no slash before the colon (this is host:port/path, not a path segment)
            let has_slash_before = s[..idx].contains('/');
            if !has_slash_before {
                s = format!("{}/{}", &s[..idx], &s[idx + pattern.len()..]);
                break;
            }
        }
    }

    // Strip trailing slash
    if s.ends_with('/') {
        s = s[..s.len() - 1].to_string();
    }

    // Strip trailing .git
    if s.ends_with(".git") {
        s = s[..s.len() - 4].to_string();
    }

    // Handle standard SSH alternate syntax: replace the first colon after the host with a slash,
    // but ONLY if there are no slashes before it (scp-like syntax, e.g., host:owner/repo)
    if let Some(colon_idx) = s.find(':') {
        let has_slash_before_colon = s
            .find('/')
            .map(|slash_idx| slash_idx < colon_idx)
            .unwrap_or(false);
        if !has_slash_before_colon {
            s.replace_range(colon_idx..colon_idx + 1, "/");
        }
    }

    // Replace any remaining colons (e.g., non-standard port numbers) with slashes
    s = s.replace(':', "/");

    s
}

pub fn inherited_ssh_binary() -> Result<String> {
    if let Some(global) = get_global_ssh_command()? {
        let candidate = first_token(&global);
        if !candidate.is_empty() {
            // Check if it's our wrapper command and avoid self-referencing loop.
            // A wrapper generated by git-sshkey contains "IdentitiesOnly=yes".
            // Skip it so we find the actual inherited ssh binary, not our own wrapper.
            if !global.contains("IdentitiesOnly=yes") {
                return Ok(candidate);
            }
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

    if let Some(stripped) = trimmed.strip_prefix('"')
        && let Some(end) = stripped.find('"')
    {
        return stripped[..end].to_string();
    }

    if let Some(stripped) = trimmed.strip_prefix('\'')
        && let Some(end) = stripped.find('\'')
    {
        return stripped[..end].to_string();
    }

    trimmed
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_token() {
        assert_eq!(first_token(""), "");
        assert_eq!(first_token("ssh -i"), "ssh");
        assert_eq!(first_token("\"/usr/bin/ssh\" -i"), "/usr/bin/ssh");
        assert_eq!(first_token("'/usr/bin/ssh' -i"), "/usr/bin/ssh");
    }

    #[test]
    fn test_normalize_remote_url() {
        let cases = vec![
            ("git@github.com:foo/bar.git", "github.com/foo/bar"),
            ("git@github.com:foo/bar", "github.com/foo/bar"),
            ("https://github.com/foo/bar.git", "github.com/foo/bar"),
            ("https://github.com/foo/bar", "github.com/foo/bar"),
            // Standard port 22 is stripped so ssh:// with port matches scp-style
            ("ssh://git@github.com:22/foo/bar.git", "github.com/foo/bar"),
            // Non-standard port is preserved
            (
                "ssh://git@github.com:2222/foo/bar.git",
                "github.com/2222/foo/bar",
            ),
            // HTTPS port 443 is stripped
            ("https://github.com:443/foo/bar.git", "github.com/foo/bar"),
            // Git protocol port 9418 is stripped
            ("git://github.com:9418/foo/bar.git", "github.com/foo/bar"),
            ("GITHUB.com:foo/bar.git", "github.com/foo/bar"),
            ("https://user@github.com/foo/bar.git", "github.com/foo/bar"),
            ("git@github.com:foo/bar.git/", "github.com/foo/bar"),
            ("ssh://git@github.com/foo/bar.git", "github.com/foo/bar"),
        ];

        for (input, expected) in cases {
            assert_eq!(
                normalize_remote_url(input),
                expected,
                "failed for: {}",
                input
            );
        }
    }
}
