# SSH & Connection Flow Protocols

This document details the low-level protocols, keyring bindings, terminal streaming channels, and SFTP synchronization mechanics implemented by Demeteo to manage secure connections with remote environments.

---

## 🔒 1. Keyring Integration & Credentials Security

To guarantee security, SSH key passphrases and machine connection passwords must **never** be stored in plaintext in the SQLite database:
1. The app reads the system's native credentials manager (macOS Keychain, Windows Credential Manager, Linux gnome-keyring) using the Rust `keyring` crate.
2. Local private keys (`~/.ssh/id_rsa`, `~/.ssh/id_ed25519`) are loaded, and the app leverages the system `ssh-agent` whenever `auth_type` is set to `'agent'`.

---

## 📺 2. Bi-directional Terminal Streaming (Tauri Channels)

For active shell terminals:
1. The frontend spawns an `xterm.js` instance.
2. The frontend invokes the Rust command `start_terminal_session` passing a Tauri v2 `Channel` and the target machine ID.
3. The Rust backend establishes the SSH connection, spawns a remote pseudo-terminal (PTY) shell, and attaches the write end to the PTY.
4. Input typed in `xterm.js` is sent via the Channel write stream to the PTY.
5. PTY output is pushed in real-time back through the Channel read stream to the frontend terminal display.

---

## 🔌 3. SSH Port Forwarding & Agent APIs

When an agent session (such as Ollama on port `11434` or Hermes on port `8000`) is opened on a remote machine:
1. The Rust backend spins up a local TCP listener on an available port on the host machine (e.g. `localhost:35000`).
2. Rust listens for connections on this port and tunnels the raw TCP stream over the active SSH session directly to the target port on the remote machine (e.g., `remote:8000`).
3. The React app is notified of the mapped local port (`35000`) and directs all REST/WebSocket API requests to `http://localhost:35000`, communicating with the remote agent securely as if it were local.

---

## 📁 4. SFTP File Explorer & Editing

To facilitate remote coding sessions:
1. **SFTP client**: The Rust backend mounts an SFTP subsystem on the remote host over the active SSH connection.
2. **Directory Listing**: The frontend queries folder hierarchies via SFTP, rendering a collapsible directory tree.
3. **File Syncing**:
   * Opening a file reads its content via SFTP and streams it to the Monaco Editor workspace tab.
   * Saving a file writes the buffer back to the remote server over SFTP.
   * Before saving, a quick modified-check is performed to warn the developer if files changed externally.
