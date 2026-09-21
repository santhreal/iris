# iris packaging contract

Shared facts every installer, the updater, and CI rely on. Do not
deviate without updating this file and every consumer.

## Identity
- Crate / binary name: `iris` (`iris.exe` on Windows)
- App display name: `iris`
- Bundle / app id: `dev.iris.app`
- Version source: `Cargo.toml` `[package].version` (currently `0.1.0`)
- Repo: `https://github.com/santhreal/iris`
- Release tag format: `v{version}` (e.g. `v0.1.0`)
- Latest-release API: `https://api.github.com/repos/santhreal/iris/releases/latest`

## Icons (already generated in packaging/icons/)
- `iris.ico` — Windows (multi-size 16..256)
- `iris.icns` — macOS
- `iris-256.png`, `iris-512.png`, `iris-1024.png` — Linux / general
- Regenerate with `python3 packaging/gen_icons.py`

## Release asset names (CI must produce exactly these)
- `iris-{ver}-windows-x86_64-setup.exe`   NSIS installer
- `iris-{ver}-macos-universal.dmg`        DMG holding iris.app
- `iris-{ver}-linux-x86_64.AppImage`      AppImage
- `iris-{ver}-linux-x86_64.deb`           Debian package
- `iris-{ver}-linux-x86_64.rpm`           RPM package
- `iris-{ver}-linux-aarch64.AppImage`     (optional, cross)
- each asset + `.sha256` sidecar

## Install layout
- Windows: `%ProgramFiles%\iris\iris.exe`; Start Menu shortcut;
  autostart via `HKCU\...\Run` value `iris` = `iris.exe` (daemon mode).
  Uninstaller removes files, shortcuts, the Run value.
- macOS: `/Applications/iris.app`; autostart via a LaunchAgent
  `~/Library/LaunchAgents/dev.iris.app.plist` running the binary.
- Linux: `/usr/bin/iris`; `iris.desktop` in `/usr/share/applications`;
  icon in `/usr/share/icons/hicolor/*/apps/iris.png`; autostart via
  `~/.config/autostart/iris.desktop` (XDG autostart).

## Runtime model
- `iris` with no args = the app/daemon (tray + hotkeys + IPC listener).
- `iris --home`, `--capture`, `--record-window`, `--settings`,
  `--library`, `--quit`, `--version`, `--check-update`, `--update`.
- The installer registers autostart so the daemon runs at login; the
  desktop shortcut launches `iris --home` (surfaces the home window in
  the running daemon, or starts it).

## Update mechanism
- `iris --check-update`: GET latest release, compare semver to current.
- `iris --update`: download the platform asset, apply it, restart the
  daemon. Windows runs the NSIS installer silent (`/S`); macOS mounts
  the DMG and swaps the .app; Linux replaces the AppImage or runs the
  package manager for .deb/.rpm.
- Settings UI gets a "Check for updates" row + current version label.
