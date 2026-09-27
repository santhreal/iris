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
- Only a tag publishes. Running the release workflow by hand
  (`gh workflow run release.yml`) is a dry run: it runs the gate,
  builds and checks every installer, and checks the asset set.
- Latest-release API: `https://api.github.com/repos/santhreal/iris/releases/latest`

## Icons (already generated in packaging/icons/)
- `iris.ico`: Windows (multi-size 16..256)
- `iris.icns`: macOS
- `iris-256.png`, `iris-512.png`, `iris-1024.png`: Linux and general
- Regenerate with `python3 packaging/gen_icons.py`

## Release asset names (CI produces these names and no others)
- `iris-{ver}-windows-x86_64-setup.exe`   NSIS installer
- `iris-{ver}-windows-x86_64-portable.zip` zip holding `iris\iris.exe`
- `iris-{ver}-macos-universal.dmg`        DMG holding iris.app
- `iris-{ver}-linux-x86_64.AppImage`      AppImage
- `iris-{ver}-linux-x86_64.deb`           Debian package
- `iris-{ver}-linux-x86_64.rpm`           RPM package
- each asset + `.sha256` sidecar

The release workflow reads this list: its publish job fails when the
built files differ from it, when a sidecar does not match its asset,
or when an updater asset (a `pub const *ASSET` in `sys::install`) is
missing from it.

## Install layout
- Windows: per-user, `%LOCALAPPDATA%\Programs\iris\iris.exe`; Start
  Menu shortcut; autostart via `HKCU\...\Run` value `iris` =
  `"...\iris.exe" --daemon`, written on a first install and rewritten
  on an upgrade only when it exists. Uninstaller removes files,
  shortcuts, the Run value.
- Windows portable: `iris\iris.exe` wherever the zip is unpacked; no
  shortcut, no Uninstall key, and no Run value until Start at login
  writes one. Delete the folder to remove it.
- macOS: `/Applications/iris.app`; autostart via a LaunchAgent
  `~/Library/LaunchAgents/dev.iris.app.plist` running
  `iris --daemon`, written by Settings' Start at login.
- Linux (deb/rpm): `/usr/bin/iris`; `dev.iris.app.desktop` in
  `/usr/share/applications`; icon in
  `/usr/share/icons/hicolor/*/apps/iris.png`; autostart via
  `/etc/xdg/autostart/iris-autostart.desktop` (XDG autostart,
  `Exec=iris --daemon`).
- Start at login (Settings) reads and writes the account's entry:
  `$XDG_CONFIG_HOME/autostart/iris-autostart.desktop` (off over a
  package entry is `Hidden=true` there), the LaunchAgent, or the Run
  value. It is the one record of the setting.

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
- `iris --update`: download the platform asset, check its SHA-256
  against the asset's `.sha256` sidecar, apply it, restart the
  daemon. A release without the sidecar, or a download that differs
  from it, fails the update before the daemon stops; the download is
  deleted. An installed Windows iris (uninstall.exe beside iris.exe)
  starts the NSIS installer with `/S /RUN`: it waits for iris.exe to be
  free, replaces it, and starts iris. A portable Windows iris unpacks
  the portable zip beside itself, renames the running iris.exe to
  iris.exe.old, moves the new one in, and starts it; the daemon deletes
  iris.exe.old at its next start. macOS mounts the DMG and swaps its
  iris.app for the bundle iris runs from in one rename; Linux replaces
  the AppImage. A deb or rpm install downloads the release's deb or
  rpm and installs it with `apt-get install -y` or `dnf install -y`,
  run through pkexec unless iris runs as root, then starts
  `/usr/bin/iris`. A Linux iris from none of the three, a macOS iris
  outside an .app bundle, a deb or rpm install without its package
  manager or pkexec, or a portable iris in a folder it cannot write
  fails before the download and leaves the daemon running.
- Settings UI gets a "Check for updates" row + current version label.
