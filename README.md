# git-sshkey

`git-sshkey` is an ultra-fast, cross-platform Rust CLI that binds a specific SSH identity from your active `ssh-agent` to a single Git repository or commands.

It solves three common problems:

- **SSH identity exhaustion:** When your agent has multiple keys loaded, servers often reject connections after too many failed attempts.
- **Preserving global SSH binary/path behavior:** Applies local repository overrides while respecting your globally configured SSH binary.
- **Fast, offline default fallback and cache fast-path:** Keeps developer experience lightning-fast without redundant network checks or prompt fatigue.

---

## Features

- **Native Git Subcommand Workflow:** Responds directly to `git sshkey ...`
- **Smart Probing with Cache Fast-Path (`-a` / `--auto`):** Automatically probe SSH keys against the remote. Successful hits are cached locally under `~/.ssh/git-sshkey/cache/` for zero-delay operations.
- **Self-Healing Cache:** If a cached key fails (e.g. key revoked), the cache is automatically cleared and re-probed.
- **Interactive Identity Picker:** Seamless dropdown selection powered by `dialoguer` reading directly from `ssh-add -L`.
- **Global Default Identity (`git sshkey default`):** Configure a global fallback identity in git config (`git-sshkey.default`).
- **Instant Command Generation (`git sshkey command`):** Outputs the exact `core.sshCommand` string for any key or remote url instantly. If a default is set, it executes offline without prompting or network delays.
- **Local-Only `core.sshCommand` Binding:** Isolate SSH keys locally per repo.
- **Windows Path Compatibility:** Auto-normalizes Windows directory backslashes into forward slashes inside Git commands, preventing escaping/parsing bugs.

---

## Installation

Install from local source:

```bash
cargo install --path .
```

This installs `git-sshkey` into Cargo's bin directory (usually `~/.cargo/bin`), which should be on your `PATH`.

Once installed, Git will automatically resolve `git sshkey` subcommands.

---

## Commands

- `git sshkey` - Show help and usage instructions.
- `git sshkey info` - Show current environment status (inherited binary, global default, local override).
- `git sshkey list` - List identities currently loaded in `ssh-agent`.
- `git sshkey pick` - Interactively select an identity from `ssh-agent` and bind it to the local repository.
- `git sshkey default [key]` - Get, set, or interactively pick the global default fallback identity. Highlights current default in picker.
- `git sshkey command [key] [--remote <url>]` - Output the SSH command string for a specific key, remote, or current default instantly.
- `git sshkey <git-command> ...` - Run any Git command temporarily using a selected `ssh-agent` identity.
- `git sshkey run <git-command> ...` - Execute a Git command whose name conflicts with an internal `git-sshkey` subcommand.
- `git sshkey test` - Run a silent auth probe against the default remote URL.
- `git sshkey clear` - Remove local `core.sshCommand` override from the repository.

---

## Technical Details

### Smart Auto-Detection Sequence (`--auto` / `-a`)

When running with the `--auto` flag, `git-sshkey` tries to authenticate against your remote repository in the following order:

1. **Cache Lookahead:** Fetches the hashed remote URL cache key under `~/.ssh/git-sshkey/cache/`. If hit, it tries that key first (**0ms network delay**).
2. **Global Default:** If cache misses or fails, it falls back to your globally configured default key (`git-sshkey.default`).
3. **Sequence Probe:** If both miss/fail, it probes all remaining loaded keys sequentially using `git ls-remote --exit-code <url> HEAD`.
4. **Caching & Healing:** Working keys are cached. Revoked or failing keys automatically trigger cache invalidation and a fresh probe cycle.

### Public Key Isolation

When binding a key, `git-sshkey` extracts and materializes public key files dynamically under `~/.ssh/git-sshkey/`:

- `~/.ssh/git-sshkey/{comment}.pub` (unique name-based)
- `~/.ssh/git-sshkey/{comment}-{short_hash}.pub` (collision fallback)
- `~/.ssh/git-sshkey/key-{short_hash}.pub` (when comment is missing or reserved system words)

---

## License

MIT. See `LICENSE`.
