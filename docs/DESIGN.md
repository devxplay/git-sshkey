# Design Specification: git-sshkey

## 1. Executive Summary

`git-sshkey` is a high-performance Rust utility designed to resolve **SSH Identity Exhaustion** and **Path Shadowing**. It functions as a native Git subcommand that allows users to bind specific SSH identities from an active `ssh-agent` to a local repository without losing global configuration settings (especially critical for Windows paths).

## 2. CLI Specification

| Command             | Action              | Logic / ROI                                                   |
| :------------------ | :------------------ | :------------------------------------------------------------ |
| `git sshkey`        | Usage Help          | Displays help and available subcommands.                      |
| `git sshkey info`   | Environment Audit   | Shows current local override and inherited binary path.       |
| `git sshkey pick`   | Interactive Binding | Polls agent, presents UI, and writes local `core.sshCommand`. |
| `git sshkey list`   | Identity Inventory  | Lists all keys in the agent with index numbers.               |
| `git sshkey <git-command>` | Command Passthrough | Polls agent, presents UI, and runs Git with `-c core.sshCommand`. |
| `git sshkey run`    | Collision Escape    | Runs Git command names reserved by `git-sshkey`.              |
| `git sshkey test`   | Silent Probe        | Runs `git ls-remote --exit-code origin HEAD` to verify auth.  |
| `git sshkey clear`  | Revert              | Deletes local override and returns to global defaults.        |

## 3. The "Smart Inheritance" Algorithm

The core technical moat is preserving the global SSH binary path while injecting local identity flags.

1.  **Global Discovery:**
    - Query `git config --global core.sshCommand`.
    - Fallback to `C:/Windows/System32/OpenSSH/ssh.exe` on Windows or `ssh` on Unix.
2.  **Identity Selection:**
    - Invoke `ssh-add -L` to read identities from the running `ssh-agent`.
    - Parse key type + key comment for user-facing selection.
    - User selects key via `dialoguer` interactive menu.
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

## 4. Technical Stack

- **Language:** Rust
- **CLI Parser:** `clap`
- **Agent Access:** shelling out to `ssh-add -L`
- **Git Config:** shelling out to `git config` for portability
- **UI:** `dialoguer`

## 5. File Naming Convention

- **Design Doc:** `SPECIFICATION.md`
- **Main Logic:** `src/main.rs`
- **Agent Module:** `src/agent.rs`
- **Config Module:** `src/config.rs`

---

## 6. Execution Roadmap

- [ ] **Milestone 1:** Implement cross-platform `ssh-agent` polling.
- [ ] **Milestone 2:** Implement Global -> Local path inheritance logic.
- [ ] **Milestone 3:** Build `pick` interactive UI.
- [ ] **Milestone 4:** Implement `test` silent network probe.
- [ ] **Milestone 5:** Package for `x64` and `arm64` release.

## 7. Current Implementation Notes

- `list` intentionally shows index + key type + comment only (no key blob preview).
- Filename hashing is now collision-based, not default behavior.
