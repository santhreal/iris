# Installation

Download release binaries and installers from repository releases:
`https://github.com/santhreal/iris/releases`. Each release asset includes a
`.sha256` checksum sidecar file.

## Release Assets

| Platform | Asset Filename | Description |
| --- | --- | --- |
| Windows (x86_64) | `iris-<ver>-windows-x86_64-setup.exe` | NSIS installer |
| macOS (Universal) | `iris-<ver>-macos-universal.dmg` | Disk image containing `iris.app` (x86_64 and aarch64) |
| Linux (x86_64) | `iris-<ver>-linux-x86_64.deb` | Debian / Ubuntu package |
| Linux (x86_64) | `iris-<ver>-linux-x86_64.rpm` | Fedora / RHEL package |
| Linux (x86_64) | `iris-<ver>-linux-x86_64.AppImage` | Standalone AppImage executable |

## Windows

Download `iris-<ver>-windows-x86_64-setup.exe`.

Execute the installer:

```cmd
iris-<ver>-windows-x86_64-setup.exe
```

For unattended installation, append the `/S` flag:

```cmd
iris-<ver>-windows-x86_64-setup.exe /S
```

To start iris when the installation ends, append `/RUN`. The installer starts iris whether the installation succeeded or failed:

```cmd
iris-<ver>-windows-x86_64-setup.exe /S /RUN
```

When iris is installed and running, the installer sends it `--quit` and waits up to 30 seconds for every process that runs `iris.exe` to exit before it replaces the file. When `iris.exe` stays in use, the installer shows a message box with **Retry** and **Cancel**. **Cancel** stops the installation, and a silent installation stops without the message box and exits with status 2.

The installer runs per-user without administrative privileges:
- Installs application files to `%LOCALAPPDATA%\Programs\iris` (`iris.exe`, `iris.ico`, `uninstall.exe`).
- Creates a Start Menu shortcut at `%APPDATA%\Microsoft\Windows\Start Menu\Programs\iris.lnk` running `iris.exe --home`.
- Registers login autostart under registry key `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, setting value `iris` to `"%LOCALAPPDATA%\Programs\iris\iris.exe" --daemon`.
- Registers uninstallation metadata under `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\iris`.

To uninstall, select **iris** in Windows Settings > Installed apps, or execute:

```cmd
"%LOCALAPPDATA%\Programs\iris\uninstall.exe"
```

The uninstaller sends `--quit` to a running iris daemon and waits for `iris.exe` in the same way, then removes installed files and shortcuts, deletes the autostart registry value, and removes application registry entries.

## macOS

Download `iris-<ver>-macos-universal.dmg`.

1. Mount the disk image:

```sh
hdiutil attach iris-<ver>-macos-universal.dmg
```

2. Copy `iris.app` into `/Applications`:

```sh
cp -R /Volumes/iris/iris.app /Applications/
hdiutil detach /Volumes/iris
```

3. Configure launch at login by creating a LaunchAgent plist:

```sh
mkdir -p ~/Library/LaunchAgents
cat << 'EOF' > ~/Library/LaunchAgents/dev.iris.app.plist
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>dev.iris.app</string>
	<key>ProgramArguments</key>
	<array>
		<string>/Applications/iris.app/Contents/MacOS/iris</string>
		<string>--daemon</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
</dict>
</plist>
EOF
launchctl load ~/Library/LaunchAgents/dev.iris.app.plist
```

To remove login autostart:

```sh
launchctl unload ~/Library/LaunchAgents/dev.iris.app.plist
rm ~/Library/LaunchAgents/dev.iris.app.plist
```

To uninstall, delete `/Applications/iris.app`.

## Linux

### Debian and Ubuntu

Download `iris-<ver>-linux-x86_64.deb`.

Install the package:

```sh
sudo apt install ./iris-<ver>-linux-x86_64.deb
```

Installed files:
- Binary: `/usr/bin/iris`
- Desktop entry: `/usr/share/applications/dev.iris.app.desktop`
- Login autostart entry: `/etc/xdg/autostart/iris-autostart.desktop`, running `iris --daemon`
- AppStream metadata: `/usr/share/metainfo/dev.iris.app.metainfo.xml`
- Icons: `/usr/share/icons/hicolor/*/apps/iris.png`
- Copyright documentation: `/usr/share/doc/iris/copyright`

To uninstall:

```sh
sudo apt remove iris
```

### Fedora and RHEL

Download `iris-<ver>-linux-x86_64.rpm`.

Install the package:

```sh
sudo dnf install ./iris-<ver>-linux-x86_64.rpm
```

Installed files:
- Binary: `/usr/bin/iris`
- Desktop entry: `/usr/share/applications/dev.iris.app.desktop`
- Login autostart entry: `/etc/xdg/autostart/iris-autostart.desktop`, running `iris --daemon`
- AppStream metadata: `/usr/share/metainfo/dev.iris.app.metainfo.xml`
- Icons: `/usr/share/icons/hicolor/*/apps/iris.png`

To uninstall:

```sh
sudo dnf remove iris
```

### AppImage

Download `iris-<ver>-linux-x86_64.AppImage`.

Set execution permissions and run:

```sh
chmod +x iris-<ver>-linux-x86_64.AppImage
./iris-<ver>-linux-x86_64.AppImage
```

## Build Requirements and Compilation

Compiling iris from source requires Rust 1.90 or newer and `cargo`.

### System Build Dependencies

#### Ubuntu / Debian

```sh
sudo apt-get install -y \
  build-essential pkg-config cmake clang \
  libasound2-dev libfontconfig-dev libglib2.0-dev libssl-dev \
  libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev libxcb1-dev \
  libvulkan1 libegl1-mesa-dev libgl1-mesa-dev \
  libpipewire-0.3-dev libspa-0.2-dev xdg-desktop-portal
```

#### Fedora / RHEL

```sh
sudo dnf install -y \
  gcc gcc-c++ make cmake clang pkgconf-pkg-config \
  alsa-lib-devel fontconfig-devel glib2-devel openssl-devel \
  wayland-devel libxcb-devel libxkbcommon-x11-devel \
  vulkan-loader mesa-libEGL-devel mesa-libGL-devel \
  pipewire-devel
```

### Compile

Build the release executable:

```sh
cargo build --release --bin iris
```

The compiled executable is written to `target/release/iris` (`target/release/iris.exe` on Windows).

## Runtime Tools

iris searches for external tool binaries in directories listed in `PATH`. On macOS, iris also inspects Homebrew and MacPorts directories (`/opt/homebrew/bin`, `/usr/local/bin`, `/opt/local/bin`). On Windows, iris inspects user and machine `Path` values in the registry and default Tesseract directories (`%ProgramFiles%\Tesseract-OCR`, `%LOCALAPPDATA%\Programs\Tesseract-OCR`).

### ffmpeg

`ffmpeg` is required for video recording across all platforms and for probing NVIDIA NVENC encoder availability.

Installation instructions:
- Linux: `sudo apt install ffmpeg` on Debian and Ubuntu, `sudo dnf install ffmpeg-free` on Fedora. The deb recommends `ffmpeg` and the rpm recommends `/usr/bin/ffmpeg`; `apt` and `dnf` install recommended packages by default. The AppImage does not include ffmpeg.
- macOS: `brew install ffmpeg`
- Windows: `winget install Gyan.FFmpeg`

### tesseract

`tesseract` is required for optical character recognition (OCR) when copying text from captures.

Installation instructions:
- Linux: `sudo apt install tesseract-ocr` on Debian and Ubuntu, `sudo dnf install tesseract` on Fedora. The deb and the rpm suggest it; `apt` and `dnf` do not install a suggested package by default.
- macOS: `brew install tesseract`
- Windows: `winget install UB-Mannheim.TesseractOCR`

## Updates

Check for newer releases:

```sh
iris --check-update
```

Download and apply the latest release:

```sh
iris --update
```

`--update` downloads the matching platform asset, sends `--quit` to any running daemon, waits up to 60 seconds for it to exit, applies the replacement file, and relaunches the executable. A quitting daemon saves its recording before it exits. When the daemon still runs 60 seconds after `--quit`, `iris --update` installs nothing, prints `update: the running iris still answers 60 s after --quit; nothing was installed, run iris --update again once it exits`, and exits with status 1.

Platform update mechanisms:
- Windows: Downloads `windows-x86_64-setup.exe`, starts it with `/S /RUN` as a detached process that receives no handle of the `iris --update` process, and exits. The installer waits until no process runs the installed `iris.exe`, replaces it, and starts iris. When the installation fails, it starts the `iris.exe` already in place.
- macOS: Downloads `macos-universal.dmg`. Attaches disk image with `hdiutil attach -nobrowse -readonly`, deletes `/Applications/iris.app`, copies new bundle via `cp -R`, detaches volume with `hdiutil detach`, and relaunches `/Applications/iris.app/Contents/MacOS/iris`.
- Linux (AppImage): Overwrites the file defined in `$APPIMAGE` via a temporary sibling file and atomic rename, then launches the new file.
- Linux (deb/rpm): When a newer release exists, `iris --update` outside an AppImage prints `update: not an AppImage install; update via apt/dnf` and exits with status 1 before downloading anything. The running daemon keeps running.

The [settings window](configuration.md#settings-window) (`iris --settings`) includes a **Check for updates** button in the Updates section that queries releases asynchronously and reports status.
