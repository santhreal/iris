# iris packaging contract

Shared facts every installer, the updater, and CI rely on. Do not
deviate without updating this file and every consumer.

## Identity
- Crate / binary name: `iris` (`iris.exe` on Windows)
- App display name: `iris`
- Bundle / app id: `dev.iris.app`
- Version source: `Cargo.toml` `[package].version` (currently `0.1.0`)
- Repo: `https://github.com/santhreal/iris`
- Release tag format: `v{version}` (e.g. `v0.1.0`, `v0.2.0-beta.1`).
  The release workflow fails on a tag that is not `v` plus the
  Cargo.toml version (the updater compares the tag with the version the
  binary reports), on a tag whose commit is not reachable from
  `origin/main`, and when CHANGELOG.md has no `## [{version}] -
  YYYY-MM-DD` section. That section is the release body. A version
  with a prerelease (`0.2.0-beta.1`) publishes as a GitHub prerelease.
- Only a tag publishes. Running the release workflow by hand
  (`gh workflow run release.yml`) is a dry run: it runs the gate and
  the cross-platform tests, builds and install-checks every installer,
  checks the asset set, signs every asset with the release key, and
  verifies every signature. It creates no tag, release, or draft.
- Latest-release API: `https://api.github.com/repos/santhreal/iris/releases/latest`

## Icons (already generated in packaging/icons/)
- `iris.ico`: Windows (multi-size 16..256)
- `iris.icns`: macOS
- `iris-{16,24,32,48,64,128,256,512,1024}.png`: Linux hicolor icons,
  installed by the deb, the rpm and the AppImage at every size
- Regenerate with `python3 packaging/gen_icons.py`

## Release asset names (CI produces these names and no others)
`{ver}` is the Cargo.toml version, prerelease included
(`iris-0.2.0-beta.1-linux-x86_64.deb`). `{arch}` is
`std::env::consts::ARCH` of the build: `x86_64` or `aarch64`.
- `iris-{ver}-windows-x86_64-setup.exe`    NSIS installer
- `iris-{ver}-windows-x86_64-portable.zip` zip holding `iris\iris.exe`
- `iris-{ver}-windows-aarch64-setup.exe`   NSIS installer, ARM64
- `iris-{ver}-windows-aarch64-portable.zip` zip holding `iris\iris.exe`, ARM64
- `iris-{ver}-macos-universal.dmg`         DMG holding iris.app (x86_64 and arm64)
- `iris-{ver}-linux-x86_64.AppImage`       AppImage
- `iris-{ver}-linux-x86_64.deb`            Debian package, `amd64`
- `iris-{ver}-linux-x86_64.rpm`            RPM package, `x86_64`
- `iris-{ver}-linux-aarch64.AppImage`      AppImage, ARM64
- `iris-{ver}-linux-aarch64.deb`           Debian package, `arm64`
- `iris-{ver}-linux-aarch64.rpm`           RPM package, `aarch64`

Every asset has two sidecars:
- `<asset>.sha256`: the output of `sha256sum <asset>`, the hex digest,
  two spaces, and the asset file name.
- `<asset>.minisig`: the minisign signature of the asset (see
  Signatures).

The release workflow reads this list: its publish job fails when the
built files differ from the assets, their `.sha256` and their
`.minisig`, when a sidecar does not match its asset, when a signature
does not verify, or when an updater asset (a `pub const *ASSET` in
`sys::install`, with `{arch}` as `x86_64` and as `aarch64`) is missing
from it.

The aarch64 assets are built and install-checked on ARM64 machines
(`ubuntu-24.04-arm`, `windows-11-arm`).

## Signatures
- Public key: `packaging/minisign.pub`, key id `EC8B4C1097043E88`.
  The updater embeds it.
- The release workflow's publish job signs each asset after the asset
  set check:
  `minisign -S -s <key> -m <asset> -t "file:<asset> version:<ver>"`.
- The trusted comment is `file:<asset file name> version:<cargo version>`,
  for example `file:iris-0.2.0-beta.1-linux-aarch64.deb version:0.2.0-beta.1`.
- Verify an asset with
  `minisign -V -p packaging/minisign.pub -m <asset>`; the signature is
  read from `<asset>.minisig`.

## Versions
The packaging scripts derive every version from the cargo version
(`packaging/lib.sh`):

| Form | Example | Where |
|------|---------|-------|
| cargo (`iris_cargo_version`) | `0.2.0-beta.1` | asset names, `iris --version`, Windows ProductVersion and FileVersion strings, the installer's DisplayVersion, the macOS `IrisVersion` key |
| tilde (`iris_tilde_version`) | `0.2.0~beta.1` | deb and rpm `Version`: `~` sorts a prerelease before its release |
| core (`iris_core_version`) | `0.2.0` | macOS `CFBundleShortVersionString` and `CFBundleVersion` |
| core + `.0` | `0.2.0.0` | Windows numeric versions: iris.exe FILEVERSION and PRODUCTVERSION (`build.rs`), the installer's VIProductVersion and VIFileVersion |

A cargo version with a `-` after its prerelease separator or in its
build metadata (`1.0.0-rc-1`, `1.0.0+build-5`) has no tilde form; the
deb and rpm builds fail on it.

## Build scripts
Each script writes its asset and the `.sha256` sidecar to `--out`. No
script signs. `--version` defaults to the Cargo.toml version.
`--ffmpeg` names a verified runtime directory holding `ffmpeg` or
`ffmpeg.exe` and `licenses/`; every executable package builder requires
it. `--arch` defaults to the architecture of the `--bin` executable
(ELF or PE); an `--arch` either executable is not built for fails. The
deb, rpm, AppImage, portable zip and DMG hold directories and
executables at mode 0755 and other files at 0644, whatever the caller's
umask.

| Script | Arguments |
|--------|-----------|
| `packaging/linux/build_deb.sh` | `--bin <path> --ffmpeg <dir> --out <dir> [--version <ver>] [--arch x86_64\|aarch64] [--skip-sha]` |
| `packaging/linux/build_rpm.sh` | `--bin <path> --ffmpeg <dir> --out <dir> [--version <ver>] [--arch x86_64\|aarch64] [--skip-sha]` |
| `packaging/linux/build_appimage.sh` | `--bin <path> --ffmpeg <dir> --out <dir> [--version <ver>] [--arch x86_64\|aarch64] [--skip-sha] [--appimagetool <path>]` |
| `packaging/windows/build_installer.sh` | `--bin <path> --ffmpeg <dir> --out <dir> [--version <ver>] [--arch x86_64\|aarch64] [--skip-sha]`, or `--check [--version <ver>] [--arch x86_64\|aarch64]` |
| `packaging/windows/build_portable.sh` | `--bin <path> --ffmpeg <dir> --out <dir> [--version <ver>] [--arch x86_64\|aarch64] [--skip-sha]` |
| `packaging/macos/make_app.sh` | `--bin <path> --ffmpeg <dir> --out <dir> [--version <ver>]` (writes `iris.app`, no sidecar) |
| `packaging/macos/make_dmg.sh` | `--app <path> --out <dir> [--version <ver>] [--skip-sha]` |

`build_appimage.sh` downloads two tools, each a tagged release checked
against a SHA-256 recorded in the script before use; other bytes fail
the build:

| Tool | Release | Files |
|------|---------|-------|
| appimagetool (runs on the build host) | AppImage/appimagetool `1.9.1` | `appimagetool-x86_64.AppImage`, `appimagetool-aarch64.AppImage` |
| type2-runtime (embedded in the AppImage) | AppImage/type2-runtime `20251108` | `runtime-x86_64`, `runtime-aarch64` |

To move a pin, download the new files, verify each runtime against its
`runtime-{arch}.sig` with the release's `signing-pubkey.asc` (key
570C77ACEA40C0F1B758902CBF96CCA56490F695), and change the version and
every hash in `build_appimage.sh` together.

## FFmpeg runtime

Every package ships FFmpeg and requires no external FFmpeg
installation. `packaging/linux/build_ffmpeg.sh --out <dir>` builds the Linux
runtime from pinned FFmpeg, x264, libvpx, Opus, and NVENC header
sources. x264, libvpx, and Opus are static; PulseAudio and zlib are
declared system libraries. `packaging/macos/build_ffmpeg.sh --out
<dir>` builds a static runtime from pinned sources on a native arm64
or x86_64 Mac. The package workflow builds and checks both Mac
architectures and combines them with `lipo`. iconv and zlib use the
macOS system libraries.
`packaging/windows/build_ffmpeg.sh --arch x86_64|aarch64 --out
<dir>` cross-builds the Windows runtime on Linux from the same pinned
sources plus zlib, with a pinned LLVM-MinGW toolchain. The package
workflow runs the resulting executable natively on each Windows
architecture before packaging it.

Run `packaging/check_ffmpeg.sh <ffmpeg path> linux|macos|windows` on
the build host and on the installed or extracted package. It rejects
`nonfree` builds, checks the encoders and capture devices iris uses,
and encodes and decodes real Matroska, MP4, WebM, and GIF files. Each runtime
directory includes upstream notices and `FFMPEG-SOURCE.txt`, which
records source and build provenance. Windows and macOS packages include
`compiler-rt.txt` and `compiler-rt-CREDITS.txt` for Clang builtins.
Windows packages also include `LLVM.txt` and `mingw-w64-runtime.txt`.
The compiler-rt inputs are in `packaging/licenses/`; their upstream
revision and SHA-256 values are in `FFMPEG-SOURCE.txt`.

## License notices
iris is MIT OR Apache-2.0. The binary embeds the Inter font
(`assets/fonts/Inter-*.ttf`), licensed under the SIL Open Font License
1.1 (`assets/fonts/Inter-OFL.txt`). Every package ships
`LICENSE-MIT`, `LICENSE-APACHE`, and `Inter-OFL.txt`:
- deb: `/usr/share/doc/iris/copyright` (copyright format 1.0) with a
  `Files: assets/fonts/Inter-*` stanza under `License: OFL-1.1`, and
  the full text of MIT, Apache-2.0, and OFL-1.1
- rpm: `%license`, in `/usr/share/licenses/iris/`
- AppImage: `usr/share/doc/iris/` in the AppDir
- Windows installer: beside `iris.exe` in the install directory
- Windows portable zip: `iris\` beside `iris.exe`
- macOS: `iris.app/Contents/Resources/`

FFmpeg and its notices ship at:
- deb: `/usr/lib/iris/ffmpeg` and `/usr/share/doc/iris/ffmpeg/`
- rpm: `%{_libdir}/iris/ffmpeg` and `%{_licensedir}/iris/ffmpeg/`
- AppImage: `usr/bin/ffmpeg` and `usr/share/doc/iris/ffmpeg/`
- Windows: `ffmpeg.exe` beside `iris.exe` and `licenses\ffmpeg\`
- macOS: `Contents/MacOS/ffmpeg` and `Contents/Resources/ffmpeg/`

## Install layout
- Windows: per-user, `%LOCALAPPDATA%\Programs\iris\iris.exe` and
  `ffmpeg.exe` beside it on x86_64 and ARM64; Start Menu shortcut; a
  desktop shortcut from the Finish page checkbox or `/DESKTOP`, rewritten
  on an upgrade when it exists; autostart via `HKCU\...\Run` value
  `iris` = `"...\iris.exe" --background`, written on a first install and
  rewritten on an upgrade only when it exists. The installer writes
  HKCU in the native (64-bit) registry view. The ARM64 installer stops
  on a PC that is not ARM64; the x86_64 installer stops on a PC that is
  neither x64 nor ARM64. Uninstaller removes files, shortcuts, the Run
  value.
- Windows portable: `iris\iris.exe` and `iris\ffmpeg.exe` wherever the
  zip is unpacked; no
  shortcut, no Uninstall key, and no Run value until Start at login
  writes one. Delete the folder to remove it.
- macOS: `/Applications/iris.app`, with `Contents/MacOS/ffmpeg`;
  autostart via a LaunchAgent
  `~/Library/LaunchAgents/dev.iris.app.plist` running
  `iris --background`, written by Settings' Start at login.
- Linux (deb/rpm): `/usr/bin/iris`; FFmpeg at `/usr/lib/iris/ffmpeg`
  for deb or `%{_libdir}/iris/ffmpeg` for rpm;
  `dev.iris.app.desktop` in
  `/usr/share/applications`; icon in
  `/usr/share/icons/hicolor/*/apps/iris.png`; autostart via
  `/etc/xdg/autostart/iris-autostart.desktop` (XDG autostart,
  `Exec=iris --background`).
- Linux (AppImage): one file, wherever it is kept. Its `AppRun` runs
  `usr/bin/iris` from the mount with the environment it was given.
  FFmpeg is `usr/bin/ffmpeg` beside iris. AppRun sets no
  `LD_LIBRARY_PATH`, `PATH` or `XDG_DATA_DIRS` entry. No desktop entry
  and no autostart until Start at login writes one.
- Start at login (Settings) reads and writes the account's entry:
  `$XDG_CONFIG_HOME/autostart/iris-autostart.desktop` (off over a
  package entry is `Hidden=true` there), the LaunchAgent, or the Run
  value. It is the one record of the setting.

## Runtime model
- `iris` with no args = the app (tray + hotkeys + IPC listener) with
  its home window, or the home window of the iris that runs. Every
  launcher entry (application menu, Start menu, desktop shortcut, the
  installer's Finish page, iris.app) runs it with no args.
- `iris --background` = the app with no window; exits when iris runs or
  starts. The autostart entries run it, so a login with iris already
  running opens no window.
- `iris --home`, `--capture`, `--record-window`, `--settings`,
  `--library`, `--quit`, `--version`, `--check-update`, `--update`.
- The installer registers autostart so iris runs at login.

## Update mechanism
- `iris --check-update`: GET latest release, compare semver to current.
- `iris --update`: download the asset for this platform and
  architecture, check its SHA-256 against the asset's `.sha256`
  sidecar and its signature against the asset's `.minisig` with the
  embedded `packaging/minisign.pub`, apply it, restart iris. The
  signature's trusted comment must be `file:<the asset> version:<the
  release version>`. A release without either sidecar, a download that
  differs from the `.sha256`, or a signature that does not verify or
  whose trusted comment holds another file or version fails the
  update before iris stops; the download is deleted. An
  installed Windows iris
  (uninstall.exe beside iris.exe) starts the NSIS installer with
  `/S /RUN`: it waits for iris.exe and ffmpeg.exe to be free, replaces
  the payload, and starts iris. A portable Windows iris unpacks the
  portable zip beside itself, replaces every packaged file with
  rollback on failure, renames the running iris.exe to iris.exe.old
  last, and starts it; iris deletes iris.exe.old and the staging
  directory at its next start.
  macOS mounts the DMG and swaps its iris.app for the bundle iris runs
  from in one rename; Linux replaces the AppImage file at `$APPIMAGE`
  when iris runs from inside `$APPDIR`, its mount. A deb or rpm
  install downloads the release's deb or rpm and installs it with
  `apt-get install -y` or `dnf install -y`, run through pkexec unless
  iris runs as root, then starts `/usr/bin/iris`. A Linux iris from
  none of the three, an AppImage or a portable iris in a folder it
  cannot write, a macOS iris outside an .app bundle, or a deb or rpm
  install without its package manager or pkexec fails before the
  download and leaves iris running.
- Settings UI gets a "Check for updates" row + current version label.
