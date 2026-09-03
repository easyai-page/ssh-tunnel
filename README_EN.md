# SSH Tunnel

[中文](README.md)

A cross-platform desktop GUI for SSH port forwarding. Manage SSH tunnels visually — local, remote, and SOCKS5 forwards — lives in the system tray, credentials stay in the OS keychain.

![Main window](docs/screenshots/main.png)

## Features

- **Three forward types**: local `-L`, remote `-R`, and dynamic `-D` (SOCKS5 proxy)
- **Flexible auth**: password, private key file (with `~` expansion and a file picker), or pasted key content
- **Credential safety**: passwords and passphrases live in the OS keychain (Windows Credential Manager / macOS Keychain / Linux Secret Service); pasted keys are stored as separate permission-locked files — no plaintext secrets in the config file
- **Host key verification**: fingerprint confirmation on first connect, prominent warning on key change (anti MITM)
- **Auto-reconnect**: exponential backoff (1s up to 30s cap), remote forwards restored after reconnect
- **System tray**: closing the window minimizes to tray; toggle forwards straight from the tray menu
- **Launch at login / auto-start forwards**: your tunnels come up with the system
- **Built-in log panel**: live connection and forward events

## Install

Download the installer for your platform from [Releases](https://github.com/easyai-page/ssh-tunnel/releases):

| Platform | File |
| --- | --- |
| Windows | `*_x64-setup.exe` / `*_arm64-setup.exe` |
| macOS | `*_x64.dmg` (Intel) / `*_aarch64.dmg` (Apple Silicon) |
| Linux | `.deb` / `.rpm` / `.AppImage` |

## Usage

1. **Add a server**: host, username, and an auth method. Secrets go to the OS keychain or permission-locked files — the config file only holds host info

   ![Add server](docs/screenshots/add-server.png)

2. **Add a forward**: pick a type, fill in listen and target addresses, then flip the switch
3. **Tray resident**: closing the window doesn't quit; the tray menu toggles forwards and reopens the window

The settings page offers auto-reconnect, minimize-to-tray, and launch-at-login toggles; the logs page streams connection events:

![Settings](docs/screenshots/settings.png)
![Logs](docs/screenshots/logs.png)

## Tech stack

- **Shell**: Tauri 2 (system tray, native installers, autostart, file dialogs)
- **Frontend**: Vue 3 + Element Plus + Pinia + Vite
- **Core** (`core/`, pure Rust, no Tauri dependency): russh (SSH protocol), tokio (one actor per server owns the connection lifecycle), keyring (OS keychain)

## Development

```bash
pnpm install
pnpm tauri dev        # desktop dev mode (hot reload)
pnpm dev              # frontend only — the browser build falls back to demo mock data
pnpm test             # frontend tests
cargo test            # Rust core tests
```

## Release

Push a `v*` tag to trigger GitHub Actions: six platform bundles are built and attached to a new Release automatically.

## License

Apache-2.0
