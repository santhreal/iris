# iris packaging contract

Shared facts every installer, the updater, and CI rely on. Do not
deviate without updating this file and every consumer.

## Identity
- Crate / binary name: `iris` (`iris.exe` on Windows)
- App display name: `iris`
- Bundle / app id: `dev.iris.app`
- Version source: `Cargo.toml` `[package].version` (currently `0.1.0`)
- Repo: `https://github.com/santhreal/iris`
- Release tag format: `v{version}` (e.g. `v0.1.0`). The release
  workflow's gate fails on a tag that is not `v` plus the Cargo.toml
  version: the updater compares the tag with the version the binary
  reports.
- Latest-release API: `https://api.github.com/repos/santhreal/iris/releases/latest`

## Icons (already generated in packaging/icons/)
- `iris.ico`: Windows (multi-size 16..256)
- `iris.icns`: macOS
- `iris-256.png`, `iris-512.png`, `iris-1024.png`: Linux and general
- Regenerate with `python3 packaging/gen_icons.py`

## Release asset names (CI produces these names and no others)
- `iris-{ver}-windows-x86_64-setup.exe`   NSIS installer
- `iris-{ver}-macos-universal.dmg`        DMG holding iris.app
- `iris-{ver}-linux-x86_64.AppImage`      AppImage
- `iris-{ver}-linux-x86_64.deb`           Debian package
- `iris-{ver}-linux-x86_64.rpm`           RPM package
- each asset + `.sha256` sidecar

## Install layout
- Windows: per-user, `%LOCALAPPDATA%\Programs\iris\iris.exe`; Start
  Menu shortcut; autostart via `HKCU\...\Run` value `iris` =
  `"...\iris.exe" --daemon`. Uninstaller removes files, shortcuts, the
  Run value.
- macOS: `/Applications/iris.app`; autostart via a LaunchAgent
  `~/Library/LaunchAgents/dev.iris.app.plist` running
  `iris --daemon`, installed by hand (`docs/install.md`).
- Linux (deb/rpm): `/usr/bin/iris`; `dev.iris.app.desktop` in
  `/usr/share/applications`; icon in
  `/usr/share/icons/hicolor/*/apps/iris.png`; autostart via
  `/etc/xdg/autostart/iris-autostart.desktop` (XDG autostart,
  `Exec=iris --daemon`).

## Runtime model
- `iris` with no args = the app/daemon (tray + hotkeys + IPC listener),
  or the home window of the daemon that runs.
- `iris --daemon` = the daemon; exits when a daemon runs or starts. The
  autostart entries run it, so a login with a daemon already running
  opens no window.
- `iris --home`, `--capture`, `--record-window`, `--settings`,
  `--library`, `--quit`, `--version`, `--check-update`, `--update`.
- The installer registers autostart so the daemon runs at login; the
  desktop shortcut launches `iris --home` (shows the home window in
  the running daemon, or starts it).

## Update mechanism
- `iris --check-update`: GET latest release, compare semver to current.
- `iris --update`: download the platform asset, apply it, restart the
  daemon. Windows starts the NSIS installer with `/S /RUN`: it waits
  for iris.exe to be free, replaces it, and starts iris; macOS mounts
  the DMG and swaps the .app; Linux replaces the AppImage. A deb or
  rpm install fails before the download and leaves the daemon
  running: the package manager updates it.
- Settings UI gets a "Check for updates" row + current version label.
