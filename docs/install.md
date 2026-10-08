# Installation

Download release binaries and installers from repository releases:
`https://github.com/santhreal/iris/releases`. Each release asset has two sidecar files beside it: `<asset>.sha256`, its SHA-256 checksum, and `<asset>.minisig`, its minisign signature ([Verifying a Download](#verifying-a-download)).

## Release Assets

`<ver>` is the release version, such as `0.2.0` or `0.2.0-beta.1`. `<arch>` is the processor architecture, `x86_64` or `aarch64`:
- Linux: `uname -m` prints it.
- Windows: Settings > System > About lists the **System type**: `x64-based processor` is `x86_64`, and `ARM-based processor` is `aarch64`.

| Platform | Asset Filename | Description |
| --- | --- | --- |
| Windows (x86_64) | `iris-<ver>-windows-x86_64-setup.exe` | NSIS installer |
| Windows (x86_64) | `iris-<ver>-windows-x86_64-portable.zip` | Portable zip holding `iris\iris.exe` |
| Windows (aarch64) | `iris-<ver>-windows-aarch64-setup.exe` | NSIS installer for Windows on Arm |
| Windows (aarch64) | `iris-<ver>-windows-aarch64-portable.zip` | Portable zip holding `iris\iris.exe` for Windows on Arm |
| macOS (Universal) | `iris-<ver>-macos-universal.dmg` | Disk image containing `iris.app` (x86_64 and aarch64) |
| Linux (x86_64) | `iris-<ver>-linux-x86_64.deb` | Debian / Ubuntu package (`amd64`) |
| Linux (x86_64) | `iris-<ver>-linux-x86_64.rpm` | Fedora / RHEL package |
| Linux (x86_64) | `iris-<ver>-linux-x86_64.AppImage` | AppImage executable using the system's libraries |
| Linux (aarch64) | `iris-<ver>-linux-aarch64.deb` | Debian / Ubuntu package (`arm64`) |
| Linux (aarch64) | `iris-<ver>-linux-aarch64.rpm` | Fedora / RHEL package |
| Linux (aarch64) | `iris-<ver>-linux-aarch64.AppImage` | AppImage executable using the system's libraries |

A release whose version has a prerelease part, such as `0.2.0-beta.1`, is a GitHub prerelease. [Updates](updates.md) describes the `beta` update channel that installs prereleases.

## Verifying a Download

The minisign public key of the releases is `packaging/minisign.pub` in the repository, key id `EC8B4C1097043E88`:

```
RWSIPgSXEEyL7E0nxtQR83kxxdTJGm0GcQ1/J/EOUgG+cCDn9RJjIYox
```

Download the asset, its two sidecars, and the public key into one directory:

```sh
ver=0.2.0
asset=iris-$ver-linux-x86_64.deb
base=https://github.com/santhreal/iris/releases/download/v$ver
curl -fL --remote-name-all "$base/$asset" "$base/$asset.sha256" "$base/$asset.minisig" \
  https://raw.githubusercontent.com/santhreal/iris/main/packaging/minisign.pub
```

Check the checksum and the signature:

```sh
sha256sum -c "$asset.sha256"
minisign -Vm "$asset" -p minisign.pub
```

`sha256sum` prints `<asset>: OK`. `minisign` prints `Signature and comment signature verified` and the trusted comment `file:<asset> version:<ver>`. Install the asset only when both checks pass and the trusted comment holds the asset's file name and the version you downloaded.

On macOS, check the checksum with `shasum -a 256 -c "$asset.sha256"`. Install minisign with `sudo apt install minisign` on Debian and Ubuntu, `sudo dnf install minisign` on Fedora, or `brew install minisign` on macOS.

On Windows, install minisign with `winget install --id jedisct1.minisign -e`, and check the files in PowerShell:

```powershell
$asset = "iris-0.2.0-windows-x86_64-setup.exe"
(Get-FileHash $asset -Algorithm SHA256).Hash.ToLower() -eq (Get-Content "$asset.sha256").Split(" ")[0]
minisign -Vm $asset -p minisign.pub
```

The first command prints `True` when the checksum matches.

## Windows

Download `iris-<ver>-windows-<arch>-setup.exe`. The aarch64 installer stops on a PC that is not ARM64. The x86_64 installer also installs on Windows 11 on Arm, which runs x64 programs under emulation; the aarch64 build runs natively.

Execute the installer:

```cmd
iris-<ver>-windows-<arch>-setup.exe
```

For unattended installation, append the `/S` flag:

```cmd
iris-<ver>-windows-<arch>-setup.exe /S
```

To start iris when the installation ends, append `/RUN`. The installer starts iris whether the installation succeeded or failed:

```cmd
iris-<ver>-windows-<arch>-setup.exe /S /RUN
```

When iris is installed and running, the installer sends it `--quit` and waits up to 30 seconds for every process that runs `iris.exe` to exit before it replaces the file. When `iris.exe` stays in use, the installer shows a message box with **Retry** and **Cancel**. **Cancel** stops the installation, and a silent installation stops without the message box and exits with status 2.

The installer runs per-user without administrative privileges:
- Installs application files to `%LOCALAPPDATA%\Programs\iris` (`iris.exe`, `iris.ico`, `LICENSE-MIT`, `LICENSE-APACHE`, `Inter-OFL.txt`, `uninstall.exe`).
- Creates a Start Menu shortcut at `%APPDATA%\Microsoft\Windows\Start Menu\Programs\iris.lnk` running `iris.exe --home`.
- Registers login autostart under registry key `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, setting value `iris` to `"%LOCALAPPDATA%\Programs\iris\iris.exe" --daemon`. An upgrade rewrites the value when it exists and writes none when **Start iris at login** was turned off ([Start at Login](configuration.md#start-at-login)).
- Registers uninstallation metadata under `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\iris`.

To uninstall, select **iris** in Windows Settings > Installed apps, or execute:

```cmd
"%LOCALAPPDATA%\Programs\iris\uninstall.exe"
```

The uninstaller sends `--quit` to a running iris daemon and waits for `iris.exe` in the same way, then removes installed files and shortcuts, deletes the autostart registry value, and removes application registry entries.

### Portable zip

Download `iris-<ver>-windows-<arch>-portable.zip`, unpack it to a folder your account can write, and run `iris\iris.exe`. The zip holds `iris\iris.exe`, `iris\LICENSE-APACHE`, `iris\LICENSE-MIT`, and `iris\Inter-OFL.txt`.

The portable iris creates no shortcut, no autostart registry value, and no uninstall entry. It keeps settings, captures, and logs in the same directories as an installed iris ([paths](configuration.md#iris_home-directory-override)). To start it at login, turn on **Start iris at login** in the settings window ([Start at Login](configuration.md#start-at-login)). To remove it, turn off **Start iris at login**, quit iris (`iris.exe --quit`), and delete the folder.

An `iris.exe` with `uninstall.exe` beside it updates through the installer; any other `iris.exe` is portable and updates from the portable zip ([Updates](#updates)).

### First Start on Windows

The installer and `iris.exe` have no Authenticode signature. The first time a downloaded installer, or an `iris.exe` unpacked from a downloaded zip, runs, Microsoft Defender SmartScreen shows **Windows protected your PC**:

1. Select **More info**.
2. Check that the app name is the file you downloaded and verified ([Verifying a Download](#verifying-a-download)).
3. Select **Run anyway**.

SmartScreen does not show the dialog again for that file. With Smart App Control turned on, Windows 11 blocks unsigned programs, iris included, and shows no **Run anyway** button.

## macOS

Download `iris-<ver>-macos-universal.dmg`. It runs on Intel and Apple silicon Macs.

1. Mount the disk image:

```sh
hdiutil attach iris-<ver>-macos-universal.dmg
```

2. Copy `iris.app` into `/Applications`:

```sh
cp -R /Volumes/iris/iris.app /Applications/
hdiutil detach /Volumes/iris
```

3. To start iris at login, turn on **Start iris at login** in the settings window (`/Applications/iris.app/Contents/MacOS/iris --settings`). Turning it on writes the LaunchAgent `~/Library/LaunchAgents/dev.iris.app.plist`, and turning it off deletes it ([Start at Login](configuration.md#start-at-login)).

To uninstall, turn off **Start iris at login**, quit iris (`/Applications/iris.app/Contents/MacOS/iris --quit`), and delete `/Applications/iris.app`.

### First Start on macOS

`iris.app` has an ad-hoc code signature and is not notarized. macOS quarantines an `iris.app` copied from a disk image that a web browser downloaded, and Gatekeeper blocks its first start with a dialog stating that Apple could not verify the app. To open it:

1. Open `iris.app`, and close the dialog with **Done** (**OK** on macOS 14 and earlier).
2. Open System Settings > Privacy & Security. Under **Security**, select **Open Anyway** beside the message about `iris.app`, and enter your password.
3. Open `iris.app` again, and select **Open Anyway** (**Open** on macOS 14 and earlier) in the dialog.

macOS opens that copy of `iris.app` without the dialog from then on. Alternatively, before the first start, delete the quarantine attribute:

```sh
xattr -dr com.apple.quarantine /Applications/iris.app
```

## Linux

The deb, the rpm, and the AppImage require glibc 2.35 or newer, as in Ubuntu 22.04, Debian 12, and later releases. On an older system, build iris from source ([Building](building.md)).

### Debian and Ubuntu

Download `iris-<ver>-linux-<arch>.deb`.

Install the package:

```sh
sudo apt install ./iris-<ver>-linux-<arch>.deb
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

Download `iris-<ver>-linux-<arch>.rpm`.

Install the package:

```sh
sudo dnf install ./iris-<ver>-linux-<arch>.rpm
```

Installed files:
- Binary: `/usr/bin/iris`
- Desktop entry: `/usr/share/applications/dev.iris.app.desktop`
- Login autostart entry: `/etc/xdg/autostart/iris-autostart.desktop`, running `iris --daemon`
- AppStream metadata: `/usr/share/metainfo/dev.iris.app.metainfo.xml`
- Icons: `/usr/share/icons/hicolor/*/apps/iris.png`
- License files: `/usr/share/licenses/iris/`

To uninstall:

```sh
sudo dnf remove iris
```

### AppImage

Download `iris-<ver>-linux-<arch>.AppImage`.

The AppImage holds the iris executable and uses the system's libraries: PipeWire (`libpipewire-0.3`), xkbcommon and xkbcommon-x11, XCB, fontconfig, the Wayland client library, the Vulkan loader, EGL, and GLES 2, the libraries the deb depends on. It runs iris with the environment it was started with and adds no `LD_LIBRARY_PATH`, `PATH`, or `XDG_DATA_DIRS` entry, so iris and the programs it starts load libraries from the system's library path. It mounts itself with FUSE through `fusermount3`, from the `fuse3` package. Without FUSE, run it with `--appimage-extract-and-run`.

Set execution permissions and run:

```sh
chmod +x iris-<ver>-linux-<arch>.AppImage
./iris-<ver>-linux-<arch>.AppImage
```

To start the AppImage at login, turn on **Start iris at login** in the settings window. Turning it on writes `~/.config/autostart/iris-autostart.desktop` running the AppImage's path with `--daemon` ([Start at Login](configuration.md#start-at-login)). Turn it off before moving or deleting the AppImage.

## From Source

[Building](building.md) lists the toolchain, the build dependencies per platform, and the package scripts.

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

The daemon checks for a newer release in the background and offers it in the tray menu, a notice, and the Updates pane of the settings window. `iris --update` installs it. The `stable` channel, the default, offers releases; the `beta` channel (`update_channel = "beta"`) also offers prereleases. iris installs an update only when the download matches its `.sha256` sidecar and its `.minisig` signature verifies. [Updates](updates.md) describes the background check, the channels, the checks, and how each kind of install is updated.
