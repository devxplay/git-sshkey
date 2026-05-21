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
            cmd_pick()
        }
        Some(Commands::Run { args }) => cmd_git(args),
        Some(Commands::Test) => {
            ensure_git_repo()?;
            cmd_test()
        }
        Some(Commands::Clear) => {
            ensure_git_repo()?;
            cmd_clear()
        }
        Some(Commands::Git(args)) => cmd_git(args),
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

    println!("inherited ssh binary: {inherited}");
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

fn cmd_pick() -> Result<()> {
    let selected = select_agent_key()?;
    let merged = ssh_command_for_key(selected.line.as_str(), &selected.comment)?;

    set_local_ssh_command(&merged)?;
    println!("Applied local core.sshCommand:");
    println!("{merged}");
    Ok(())
}

fn cmd_git(args: Vec<String>) -> Result<()> {
    if args.is_empty() {
        bail!("no git command provided");
    }

    let selected = select_agent_key()?;
    let merged = ssh_command_for_key(selected.line.as_str(), &selected.comment)?;

    let mut command = Command::new("git");
    command.arg("-c").arg(format!("core.sshCommand={merged}"));
    command.args(args);

    let status = command.status().context("failed to execute git")?;
    if status.success() {
        return Ok(());
    }

    bail!("git command failed (exit status: {status})");
}

fn select_agent_key() -> Result<agent::AgentKey> {
    let keys = list_agent_keys()?;
    if keys.is_empty() {
        bail!("no identities found in ssh-agent (try `ssh-add` first)");
    }

    let selections: Vec<String> = keys
        .iter()
        .map(|k| {
            if k.comment.is_empty() {
                format!("{} <no comment>", k.key_type)
            } else {
                format!("{} {}", k.key_type, k.comment)
            }
        })
        .collect();

    let idx = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select an ssh-agent identity for this repository")
        .items(&selections)
        .default(0)
        .interact()
        .context("failed to read selection")?;

    Ok(keys[idx].clone())
}

fn ssh_command_for_key(public_key_line: &str, key_comment: &str) -> Result<String> {
    let pub_path = persist_public_key(public_key_line, key_comment)?;
    let binary = inherited_ssh_binary()?;
    Ok(format!(
        "\"{}\" -i \"{}\" -o IdentitiesOnly=yes",
        binary,
        pub_path.display()
    ))
}

fn cmd_test() -> Result<()> {
    let status = Command::new("git")
        .args(["ls-remote", "--exit-code", "origin", "HEAD"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to execute git ls-remote")?;

    if status.success() {
        println!("ok: origin authentication succeeded");
        return Ok(());
    }

    bail!("origin authentication failed (exit status: {status})");
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

    fs::write(&path, format!("{public_key_line}\n"))
        .with_context(|| format!("failed writing key file `{}`", path.display()))?;
    Ok(path)
}

fn sanitized_key_name(raw: &str) -> String {
    raw.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_lowercase()
}
