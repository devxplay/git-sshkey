# Design Specification: git-sshkey

## 1. Executive Summary

`git-sshkey` is a high-performance Rust utility designed to resolve **SSH Identity Exhaustion** and **Path Shadowing**. It functions as a native Git subcommand that allows users to bind specific SSH identities from an active `ssh-agent` to a local repository without losing global configuration settings (especially critical for Windows paths).

## 2. CLI Specification

| Command                    | Action                 | Logic / ROI                                                         |
| :------------------------- | :--------------------- | :------------------------------------------------------------------ |
| `git sshkey`               | Usage Help             | Displays help and available subcommands.                            |
| `git sshkey info`          | Environment Audit      | Shows current local override, global default, and inherited binary. |
| `git sshkey pick`          | Interactive Binding    | Polls agent, presents UI, and writes local `core.sshCommand`.       |
| `git sshkey list`          | Identity Inventory     | Lists all keys in the agent with index numbers.                     |
| `git sshkey default`       | Global Default Manager | Get/set/interact with global fallback default key config.           |
| `git sshkey command`       | Command Helper         | Instantly output the SSH command string for a key or remote.        |
| `git sshkey <git-command>` | Command Passthrough    | Runs Git with temporary `-c core.sshCommand` via picker/fallback.   |
| `git sshkey run`           | Collision Escape       | Runs Git command names reserved by `git-sshkey`.                    |
| `git sshkey test`          | Silent Probe           | Runs `git ls-remote --exit-code origin HEAD` to verify auth.        |
| `git sshkey clear`         | Revert                 | Deletes local override and returns to global/system defaults.       |

---

## 3. Key Architectural Features

### A. Interactive Default Setting (`git sshkey default`)

- **Interactive Picker:** Running `git sshkey default` with no arguments presents an interactive dialoguer selector.
- **Visual Highlighter:** It checks your global git config (`git-sshkey.default`). If any active agent key matches the default identifier, it highlights it as `(current default)`.
- **Direct Setter:** Explicitly providing an argument (e.g. `git sshkey default my-key-comment`) bypasses the picker and updates the global fallback instantly (or errors if not currently loaded).

### B. Instant Command Lookup (`git sshkey command`)

- **Offline Efficiency:** Running `git sshkey command` first checks if a global default exists and is loaded. If yes, it outputs the command string instantly without network calls or prompts.
- **Interactive Fallback:** If no default key is configured or loaded, it launches the interactive picker to select a key and then prints its corresponding command.
- **Direct Matching:** Specifying `git sshkey command my-key` bypasses prompts and outputs its command.
- **Auto-probing Integration:** Specifying `--auto` runs the smart probe sequence to identify and print the command of the first working key for the current remote.

### C. Smart Probing with Cache Fast-Path (`--auto` / `-a` Flag)

When running commands with `--auto`, `git-sshkey` initiates a non-interactive probe sequence:

1. **Cache Lookahead:** Checks the local cache (`~/.ssh/git-sshkey/cache/sha256(remote_url)`). If a cached working key exists, it attempts it first. If successful, it returns with **zero delay**.
2. **Global Default Fallback:** If the cache misses or fails, it tries the configured global default key.
3. **Sequence Testing:** If both of the above fail, it probes all remaining active ssh-agent keys sequentially using `git ls-remote --exit-code <url> HEAD`.
4. **Self-Healing Cache:** Working keys are cached locally. If a cached key later fails (e.g., revoked permission), the cache is automatically invalidated and the probe is re-run.

---

## 4. The "Smart Inheritance" Algorithm

The core technical moat is preserving the global SSH binary path while injecting local identity flags.

1.  **Global Discovery:**
    - Query `git config --global core.sshCommand`.
    - Fallback to `C:/Windows/System32/OpenSSH/ssh.exe` on Windows or `ssh` on Unix.
2.  **Identity Selection & Routing:**
    - Invoke `ssh-add -L` to read identities from the running `ssh-agent`.
    - Resolve the key based on cache -> global default -> sequential testing / interactive menu.
3.  **Local Binding:**
    - Store the selected public key in `~/.ssh/git-sshkey/{comment}.pub` when unique.
    - If comment collides with a different key, fallback to `~/.ssh/git-sshkey/{comment}-{short_hash}.pub`.
    - If comment is unavailable, fallback to `~/.ssh/git-sshkey/key-{short_hash}.pub`.
    - Construct the merged command:
      `"{global_binary}" -i {pub_path} -o IdentitiesOnly=yes`
    - Execute `git config --local core.sshCommand "{merged_command}"`.
4.  **Command Passthrough:**
    - Use the same identity-selection and command-construction logic.
    - Execute `git -c core.sshCommand="{merged_command}" <git-command> ...` so the selected identity applies to one Git invocation.
    - Reserve internal `git-sshkey` command names; use `git sshkey run <git-command> ...` when a Git command collides.

## 5. Technical Stack

- **Language:** Rust
- **CLI Parser:** `clap`
- **Agent Access:** shelling out to `ssh-add -L`
- **Git Config:** shelling out to `git config` for portability
- **UI:** `dialoguer`

## 6. File Naming Convention

- **Design Doc:** `docs/DESIGN.md`
- **Main Logic:** `src/main.rs`
- **Agent Module:** `src/agent.rs`
- **Config Module:** `src/config.rs`

---

## 7. Execution Status

- [x] **Milestone 1:** Implement cross-platform `ssh-agent` polling.
- [x] **Milestone 2:** Implement Global -> Local path inheritance logic.
- [x] **Milestone 3:** Build `pick` interactive UI.
- [x] **Milestone 4:** Implement `test` silent network probe.
- [x] **Milestone 5:** Build interactive default management and `command` lookups.
- [x] **Milestone 6:** Integrate Cache Fast-Path and Self-Healing Probing.
- [x] **Milestone 7:** Cross-platform path escaping and Windows support.
