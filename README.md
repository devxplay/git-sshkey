# git-sshkey

`git-sshkey` is a Rust CLI that binds one SSH identity from your active `ssh-agent` to a single Git repository.

It solves two common problems:

- SSH identity exhaustion when your agent has multiple keys loaded
- Preserving global SSH binary/path behavior while applying a local repo override

## Features

- Native Git-style subcommand workflow (`git sshkey ...`)
- Interactive identity picker from `ssh-add -L`
- Local-only `core.sshCommand` binding
- Inherits global SSH binary path when available
- One-command auth probe against `origin`
- Safe clear/revert flow

## Installation

Install from local source (recommended):

```bash
cargo install --path .
```

This installs `git-sshkey` into Cargo's bin directory (usually `~/.cargo/bin`), which should be on your `PATH`.

If you prefer manual build/copy:

```bash
cargo build --release
cp ./target/release/git-sshkey /usr/local/bin/git-sshkey
```

Then Git resolves:

```bash
git sshkey <subcommand>
```

Update to latest local changes:

```bash
cargo install --path . --force
```

Uninstall:

```bash
cargo uninstall git-sshkey
```

## Commands

- `git sshkey` - show help
- `git sshkey status` - show inherited SSH binary and local override
- `git sshkey list` - list identities currently in `ssh-agent`
- `git sshkey pick` - interactively select an identity and bind it locally
- `git sshkey test` - run silent auth probe via `git ls-remote --exit-code origin HEAD`
- `git sshkey clear` - remove local `core.sshCommand` override

## How `pick` Works

1. Reads available identities from `ssh-add -L`
2. Lets you choose one key via interactive prompt
3. Stores selected public key at:
   - `~/.ssh/git-sshkey/{comment}.pub` when comment is unique
   - `~/.ssh/git-sshkey/{comment}-{short_hash}.pub` only on name collision
   - `~/.ssh/git-sshkey/key-{short_hash}.pub` when comment is empty
4. Builds merged command:
   - `"{inherited_ssh_binary}" -i "{pub_path}" -o IdentitiesOnly=yes`
5. Writes repository-local config:
   - `git config --local core.sshCommand "{merged_command}"`

## Requirements

- Rust toolchain (for building)
- Git
- OpenSSH client (`ssh`, `ssh-add`)
- Running `ssh-agent` with identities loaded

## License

MIT. See `LICENSE`.
