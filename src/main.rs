mod agent;
mod config;

use agent::list_agent_keys;
use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use config::{
    ensure_pub_storage_dir, get_local_ssh_command, inherited_ssh_binary, set_local_ssh_command,
    unset_local_ssh_command,
};
use dialoguer::{Select, theme::ColorfulTheme};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Parser)]
#[command(
    name = "git-sshkey",
    about = "Bind an ssh-agent identity to this repository",
    long_about = None
)]
struct Cli {
    /// Auto-detect key instead of prompting
    #[arg(long, short, global = true)]
    auto: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Show git-sshkey config and inherited SSH binary
    Info,
    /// List identities from ssh-agent
    List,
    /// Pick an identity and bind it to this repo
    Pick,
    /// Print the SSH command for a key (auto-detected or default)
    Command {
        /// Key identifier (comment, index, or fingerprint). Treated as remote hint with --auto.
        key: Option<String>,
        /// Optional remote URL or nickname (e.g. origin) for auto-detection
        #[arg(long, short)]
        remote: Option<String>,
    },
    /// Get or set the default fallback SSH key
    Default {
        /// Key identifier to set as default. Treated as remote hint with --auto. If omitted, shows picker.
        key: Option<String>,
        /// Optional remote URL or nickname to auto-detect and set as default
        #[arg(long, short)]
        remote: Option<String>,
    },
    /// Run a Git command with a selected ssh-agent identity
    Run {
        /// Git arguments to run
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Verify origin auth without output noise
    Test,
    /// Remove local core.sshCommand override
    Clear,
    /// Run an unknown Git command with a selected ssh-agent identity
    #[command(external_subcommand)]
    Git(Vec<String>),
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Info) => cmd_info(),
        Some(Commands::List) => cmd_list(),
        Some(Commands::Pick) => {
            ensure_git_repo()?;
            cmd_pick(cli.auto)
        }
        Some(Commands::Command { key, remote }) => cmd_command(key, remote, cli.auto),
        Some(Commands::Default { key, remote }) => cmd_default(key, remote, cli.auto),
        Some(Commands::Run { args }) => cmd_git(args, cli.auto),
        Some(Commands::Test) => {
            ensure_git_repo()?;
            cmd_test()
        }
        Some(Commands::Clear) => {
            ensure_git_repo()?;
            cmd_clear()
        }
        Some(Commands::Git(args)) => cmd_git(args, cli.auto),
        None => {
            // Default `git sshkey` behavior is usage help.
            let mut cmd = Cli::command();
            cmd.print_help().context("failed to print help")?;
            println!();
            Ok(())
        }
    }
}

fn cmd_info() -> Result<()> {
    let local = if is_git_repo()? {
        get_local_ssh_command()?
    } else {
        None
    };
    let inherited = inherited_ssh_binary()?;
    let global_default = config::get_global_default_key()?;

    println!("inherited ssh binary: {inherited}");
    match global_default {
        Some(value) if !value.is_empty() => println!("global default key: {value}"),
        _ => println!("global default key: <not set>"),
    }
    match local {
        Some(value) if !value.is_empty() => println!("local core.sshCommand: {value}"),
        _ => println!("local core.sshCommand: <not set>"),
    }
    Ok(())
}

fn cmd_list() -> Result<()> {
    let keys = list_agent_keys()?;
    if keys.is_empty() {
        println!("No identities found in ssh-agent.");
        return Ok(());
    }

    for (idx, key) in keys.iter().enumerate() {
        let label = if key.comment.is_empty() {
            "<no comment>"
        } else {
            &key.comment
        };
        println!("[{idx}] {} {label}", key.key_type);
    }
    Ok(())
}

fn cmd_pick(auto: bool) -> Result<()> {
    let selected = resolve_key(auto, None, None, !auto)?;
    let merged = ssh_command_for_key(selected.line.as_str(), &selected.comment)?;

    set_local_ssh_command(&merged)?;
    println!("Applied local core.sshCommand:");
    println!("{merged}");
    Ok(())
}

fn cmd_command(key_opt: Option<String>, remote_opt: Option<String>, auto: bool) -> Result<()> {
    // If --auto is specified or a remote is provided
    if auto || remote_opt.is_some() {
        let remote_arg = remote_opt.or(if auto { key_opt.clone() } else { None });
        let remote_url = match remote_arg {
            Some(ref r) if r.contains(':') || r.contains('@') => r.clone(),
            other => {
                ensure_git_repo()?;
                config::get_remote_url(other.as_deref())?.ok_or_else(|| {
                    anyhow::anyhow!("could not resolve remote URL for command auto-detection")
                })?
            }
        };
        let keys = list_agent_keys()?;
        let detected = auto_detect_key(&keys, &remote_url)?;
        let cmd = ssh_command_for_key(detected.line.as_str(), &detected.comment)?;
        println!("{cmd}");
        return Ok(());
    }

    // resolve_key handles: explicit key lookup → default key fast-path → interactive picker fallback
    // With force_interactive=false, it returns the default key silently if one is set and loaded.
    // If no default is loaded, it falls through to the interactive picker.
    let selected = resolve_key(false, key_opt.as_deref(), None, false)?;
    let cmd = ssh_command_for_key(selected.line.as_str(), &selected.comment)?;
    println!("{cmd}");
    Ok(())
}

fn cmd_default(key_opt: Option<String>, remote_opt: Option<String>, auto: bool) -> Result<()> {
    // Scenario 1: --auto is provided (or a remote is specified)
    if auto || remote_opt.is_some() {
        let remote_arg = remote_opt.or(if auto { key_opt.clone() } else { None });
        let remote_url = match remote_arg {
            Some(ref r) if r.contains(':') || r.contains('@') => r.clone(),
            other => {
                ensure_git_repo()?;
                config::get_remote_url(other.as_deref())?.ok_or_else(|| {
                    anyhow::anyhow!("could not resolve remote URL for default auto-detection")
                })?
            }
        };

        println!("Auto-detecting working key targeting remote: {remote_url}");
        let keys = list_agent_keys()?;
        let detected = auto_detect_key(&keys, &remote_url)?;
        let identifier = detected.identifier();

        config::set_global_default_key(&identifier)?;
        println!(
            "Set global default key to: {} ({})",
            identifier, detected.key_type
        );
        return Ok(());
    }

    let keys = list_agent_keys()?;

    // Scenario 2: A key identifier is explicitly provided
    if let Some(ref identifier) = key_opt {
        if let Some(matched) = find_key_by_identifier(&keys, identifier) {
            let id_str = matched.identifier();
            config::set_global_default_key(&id_str)?;
            println!(
                "Set global default key to: {} ({})",
                id_str, matched.key_type
            );
            return Ok(());
        } else {
            bail!("key with identifier '{identifier}' not found in active ssh-agent keys");
        }
    }

    // Scenario 3: No arguments, show the picker with the current default indicated
    if keys.is_empty() {
        bail!("no identities found in ssh-agent (try `ssh-add` first)");
    }

    let default_id_opt = config::get_global_default_key()?;
    let default_key = match default_id_opt {
        Some(ref id) if !id.is_empty() => find_key_by_identifier(&keys, id),
        _ => None,
    };

    let idx = interactive_key_picker(
        &keys,
        default_key.as_ref(),
        "Select an ssh-agent identity to set as global default",
    )?;

    let selected = &keys[idx];
    let id_str = selected.identifier();
    config::set_global_default_key(&id_str)?;
    println!(
        "Set global default key to: {} ({})",
        id_str, selected.key_type
    );
    Ok(())
}

fn cmd_git(args: Vec<String>, auto: bool) -> Result<()> {
    if args.is_empty() {
        bail!("no git command provided");
    }

    let mut remote_arg_str = None;
    for arg in &args {
        if !arg.starts_with('-')
            && (arg.contains('@') && arg.contains(':')
                || arg.starts_with("ssh://")
                || arg.starts_with("git://")
                || arg.starts_with("https://")
                || arg.starts_with("http://"))
        {
            remote_arg_str = Some(arg.clone());
            break;
        }
    }

    let mut remote_resolved_url = None;
    if let Some(ref r) = remote_arg_str {
        remote_resolved_url = Some(r.clone());
    } else if is_git_repo().unwrap_or(false) {
        for arg in &args {
            if !arg.starts_with('-')
                && let Ok(Some(url)) = config::get_remote_url(Some(arg))
            {
                remote_resolved_url = Some(url);
                break;
            }
        }
    }

    let selected = resolve_key(auto, None, remote_resolved_url.as_deref(), false)?;
    let merged = ssh_command_for_key(selected.line.as_str(), &selected.comment)?;

    let mut command = Command::new("git");
    command.arg("-c").arg(format!("core.sshCommand={merged}"));
    command.args(args);

    let status = command.status().context("failed to execute git")?;
    if status.success() {
        return Ok(());
    }

    std::process::exit(status.code().unwrap_or(1));
}

fn find_key_by_identifier(keys: &[agent::AgentKey], identifier: &str) -> Option<agent::AgentKey> {
    let id_trimmed = identifier.trim();
    if id_trimmed.is_empty() {
        return None;
    }

    for key in keys {
        if key.comment == id_trimmed {
            return Some(key.clone());
        }
    }

    for key in keys {
        if key.comment.to_lowercase() == id_trimmed.to_lowercase() {
            return Some(key.clone());
        }
    }

    if let Ok(idx) = id_trimmed.parse::<usize>()
        && idx < keys.len()
    {
        return Some(keys[idx].clone());
    }

    for key in keys {
        let mut hasher = Sha256::new();
        hasher.update(key.line.as_bytes());
        let fingerprint = hex::encode(hasher.finalize());
        if fingerprint.starts_with(id_trimmed) || id_trimmed.starts_with(&fingerprint) {
            return Some(key.clone());
        }
    }

    None
}

/// Builds a selection list annotating the current default, then runs an interactive picker.
/// Returns the chosen index into `keys`.
fn interactive_key_picker(
    keys: &[agent::AgentKey],
    default_key: Option<&agent::AgentKey>,
    prompt: &str,
) -> Result<usize> {
    let mut default_idx = 0;
    let selections: Vec<String> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let is_default = default_key.is_some_and(|def| def.line == k.line);

            let base = if k.comment.is_empty() {
                format!("{} <no comment>", k.key_type)
            } else {
                format!("{} {}", k.key_type, k.comment)
            };

            if is_default {
                default_idx = i;
                format!("{base} (current default)")
            } else {
                base
            }
        })
        .collect();

    Select::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .items(&selections)
        .default(default_idx)
        .interact()
        .context("failed to read selection")
}

fn resolve_key(
    auto: bool,
    explicit_key: Option<&str>,
    remote_arg: Option<&str>,
    force_interactive: bool,
) -> Result<agent::AgentKey> {
    let keys = list_agent_keys()?;
    if keys.is_empty() {
        bail!("no identities found in ssh-agent (try `ssh-add` first)");
    }

    if let Some(identifier) = explicit_key {
        return find_key_by_identifier(&keys, identifier).ok_or_else(|| {
            anyhow::anyhow!("key with identifier '{identifier}' not found in active ssh-agent keys")
        });
    }

    if auto {
        let remote_url = match remote_arg {
            Some(r) => r.to_string(),
            None => {
                ensure_git_repo()?;
                config::get_remote_url(None)?.ok_or_else(|| {
                    anyhow::anyhow!("could not resolve remote URL for auto-detection")
                })?
            }
        };
        return auto_detect_key(&keys, &remote_url);
    }

    let default_id = config::get_global_default_key()?;

    if !force_interactive
        && let Some(ref current) = default_id
        && !current.is_empty()
        && let Some(matched) = find_key_by_identifier(&keys, current)
    {
        return Ok(matched);
    }

    let default_key = match default_id {
        Some(ref id) if !id.is_empty() => find_key_by_identifier(&keys, id),
        _ => None,
    };

    let idx = interactive_key_picker(&keys, default_key.as_ref(), "Select an ssh-agent identity")?;

    Ok(keys[idx].clone())
}

fn probe_key(key: &agent::AgentKey, remote_url: &str) -> Result<bool> {
    let mut merged = ssh_command_for_key(key.line.as_str(), &key.comment)?;
    merged.push_str(" -o BatchMode=yes");
    let status = Command::new("git")
        .arg("-c")
        .arg(format!("core.sshCommand={merged}"))
        .args(["ls-remote", "--exit-code", remote_url, "HEAD"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to execute git ls-remote for probe")?;
    Ok(status.success() || status.code() == Some(2))
}

fn auto_detect_key(keys: &[agent::AgentKey], remote_url: &str) -> Result<agent::AgentKey> {
    if keys.is_empty() {
        bail!("no identities found in ssh-agent (try `ssh-add` first)");
    }

    let mut tried = HashSet::new();

    // 1. Try cached key first
    if let Some(cached_line) = config::get_cached_key_for_remote(remote_url)?
        && let Some(matched) = keys.iter().find(|k| k.line.trim() == cached_line.trim())
    {
        eprintln!("Trying cached key for {remote_url}...");
        tried.insert(matched.line.clone());
        if probe_key(matched, remote_url)? {
            return Ok(matched.clone());
        } else {
            eprintln!("Cached key failed, clearing cache.");
            config::clear_cached_key_for_remote(remote_url)?;
        }
    }

    // 2. Try default key
    if let Some(default_id) = config::get_global_default_key()?
        && !default_id.is_empty()
        && let Some(matched) = find_key_by_identifier(keys, &default_id)
        && !tried.contains(&matched.line)
    {
        eprintln!("Trying default key ({default_id})...");
        tried.insert(matched.line.clone());
        if probe_key(&matched, remote_url)? {
            config::set_cached_key_for_remote(remote_url, &matched.line)?;
            return Ok(matched);
        }
    }

    // 3. Try all remaining keys
    for key in keys {
        if tried.contains(&key.line) {
            continue;
        }

        let label = if key.comment.is_empty() {
            &key.key_type
        } else {
            &key.comment
        };
        eprintln!("Probing key: {label}...");
        tried.insert(key.line.clone());
        if probe_key(key, remote_url)? {
            eprintln!("Success! Key {label} is working.");
            config::set_cached_key_for_remote(remote_url, &key.line)?;
            return Ok(key.clone());
        }
    }

    bail!("no working identity found in ssh-agent for remote: {remote_url}");
}

fn ssh_command_for_key(public_key_line: &str, key_comment: &str) -> Result<String> {
    let pub_path = persist_public_key(public_key_line, key_comment)?;
    let binary = inherited_ssh_binary()?;
    let path_str = pub_path.to_string_lossy().replace('\\', "/");
    let binary_str = binary.replace('\\', "/");
    Ok(format!(
        "\"{}\" -i \"{}\" -o IdentitiesOnly=yes",
        binary_str, path_str
    ))
}

fn cmd_test() -> Result<()> {
    let remote_url = match config::get_remote_url(None)? {
        Some(url) => url,
        None => {
            bail!("no remotes configured in this repository");
        }
    };

    println!("Testing authentication against remote: {remote_url}...");
    let status = Command::new("git")
        .args(["ls-remote", "--exit-code", &remote_url, "HEAD"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to execute git ls-remote")?;

    if status.success() || status.code() == Some(2) {
        println!("ok: authentication succeeded");
        return Ok(());
    }

    bail!("authentication failed (exit status: {status})");
}

fn cmd_clear() -> Result<()> {
    unset_local_ssh_command()?;
    println!("Cleared local core.sshCommand override.");
    Ok(())
}

fn ensure_git_repo() -> Result<()> {
    if is_git_repo()? {
        Ok(())
    } else {
        bail!("current directory is not a git working tree")
    }
}

fn is_git_repo() -> Result<bool> {
    let status = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to run git rev-parse")?;

    Ok(status.success())
}

fn persist_public_key(public_key_line: &str, key_comment: &str) -> Result<PathBuf> {
    let mut hasher = Sha256::new();
    hasher.update(public_key_line.as_bytes());
    let fingerprint = hex::encode(hasher.finalize());
    let short_fingerprint = &fingerprint[..8];

    let dir = ensure_pub_storage_dir()?;
    let basename = sanitized_key_name(key_comment);
    let path = if basename.is_empty() {
        dir.join(format!("key-{short_fingerprint}.pub"))
    } else {
        let primary = dir.join(format!("{basename}.pub"));
        if !primary.exists() {
            primary
        } else {
            let existing = fs::read_to_string(&primary).unwrap_or_default();
            if existing.trim_end() == public_key_line {
                primary
            } else {
                dir.join(format!("{basename}-{short_fingerprint}.pub"))
            }
        }
    };

    // Only write the file if it doesn't exist or has different content
    let existing = fs::read_to_string(&path).ok();
    if existing.as_ref().map(|s| s.trim_end()) != Some(public_key_line) {
        fs::write(&path, format!("{public_key_line}\n"))
            .with_context(|| format!("failed writing key file `{}`", path.display()))?;
    }
    Ok(path)
}

fn sanitized_key_name(raw: &str) -> String {
    let mut result = String::new();
    let mut last_was_hyphen = false;
    for c in raw.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => {
                let mapped = if c == '_' {
                    '_'
                } else {
                    c.to_ascii_lowercase()
                };
                if mapped == '-' {
                    if !last_was_hyphen {
                        result.push('-');
                        last_was_hyphen = true;
                    }
                } else {
                    result.push(mapped);
                    last_was_hyphen = false;
                }
            }
            _ => {
                if !last_was_hyphen {
                    result.push('-');
                    last_was_hyphen = true;
                }
            }
        }
    }
    let trimmed = result.trim_matches('-').to_string();
    let reserved = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    if reserved.contains(&trimmed.as_str()) {
        String::new()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitized_key_name() {
        assert_eq!(
            sanitized_key_name("john.doe@company.com"),
            "john-doe-company-com"
        );
        assert_eq!(sanitized_key_name("My SSH Key!!!"), "my-ssh-key");
        assert_eq!(sanitized_key_name("abc--def__ghi"), "abc-def__ghi");
        assert_eq!(sanitized_key_name("---abc---"), "abc");
        assert_eq!(sanitized_key_name(""), "");
    }

    #[test]
    fn test_find_key_by_identifier_and_identifier() {
        let keys = vec![
            agent::AgentKey {
                line: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPp3rPain pain".to_string(),
                key_type: "ssh-ed25519".to_string(),
                comment: "pain".to_string(),
            },
            agent::AgentKey {
                line: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPp3rNoComment".to_string(),
                key_type: "ssh-ed25519".to_string(),
                comment: "".to_string(),
            },
            agent::AgentKey {
                line: "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDF123 admin".to_string(),
                key_type: "ssh-rsa".to_string(),
                comment: "admin".to_string(),
            },
        ];

        // Match by exact comment
        let found = find_key_by_identifier(&keys, "pain").unwrap();
        assert_eq!(found.comment, "pain");

        // Match by case-insensitive comment
        let found = find_key_by_identifier(&keys, "PAIN").unwrap();
        assert_eq!(found.comment, "pain");

        // Match by index
        let found = find_key_by_identifier(&keys, "2").unwrap();
        assert_eq!(found.comment, "admin");

        // Match by identifier (empty comment key)
        let empty_comment_key = &keys[1];
        let id = empty_comment_key.identifier();
        assert_eq!(id.len(), 8);
        let found = find_key_by_identifier(&keys, &id).unwrap();
        assert_eq!(found.line, empty_comment_key.line);

        // Match by partial fingerprint
        let found = find_key_by_identifier(&keys, &id[..4]).unwrap();
        assert_eq!(found.line, empty_comment_key.line);

        // No match
        assert!(find_key_by_identifier(&keys, "unknown").is_none());
    }
}
