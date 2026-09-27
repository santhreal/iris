# Installation

Download release binaries and installers from repository releases:
`https://github.com/santhreal/iris/releases`. Each release asset includes a
`.sha256` checksum sidecar file.

## Release Assets

| Platform | Asset Filename | Description |
| --- | --- | --- |
| Windows (x86_64) | `iris-<ver>-windows-x86_64-setup.exe` | NSIS installer |
| Windows (x86_64) | `iris-<ver>-windows-x86_64-portable.zip` | Portable zip holding `iris\iris.exe` |
| macOS (Universal) | `iris-<ver>-macos-universal.dmg` | Disk image containing `iris.app` (x86_64 and aarch64) |
| Linux (x86_64) | `iris-<ver>-linux-x86_64.deb` | Debian / Ubuntu package |
| Linux (x86_64) | `iris-<ver>-linux-x86_64.rpm` | Fedora / RHEL package |
| Linux (x86_64) | `iris-<ver>-linux-x86_64.AppImage` | AppImage executable using the system's libraries |

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
- Registers login autostart under registry key `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, setting value `iris` to `"%LOCALAPPDATA%\Programs\iris\iris.exe" --daemon`. An upgrade rewrites the value when it exists and writes none when **Start at login** was turned off ([Start at Login](configuration.md#start-at-login)).
- Registers uninstallation metadata under `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\iris`.

To uninstall, select **iris** in Windows Settings > Installed apps, or execute:

```cmd
"%LOCALAPPDATA%\Programs\iris\uninstall.exe"
```

The uninstaller sends `--quit` to a running iris daemon and waits for `iris.exe` in the same way, then removes installed files and shortcuts, deletes the autostart registry value, and removes application registry entries.

### Portable zip

Download `iris-<ver>-windows-x86_64-portable.zip`, unpack it to a folder your account can write, and run `iris\iris.exe`. The zip holds `iris\iris.exe`, `iris\LICENSE-APACHE`, and `iris\LICENSE-MIT`.

The portable iris creates no shortcut, no autostart registry value, and no uninstall entry. It keeps settings, captures, and logs in the same directories as an installed iris ([paths](configuration.md#iris_home-directory-override)). To start it at login, turn on **Start at login** in the settings window ([Start at Login](configuration.md#start-at-login)). To remove it, turn off **Start at login**, quit iris (`iris.exe --quit`), and delete the folder.

An `iris.exe` with `uninstall.exe` beside it updates through the installer; any other `iris.exe` is portable and updates from the portable zip ([Updates](#updates)).

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

3. To start iris at login, turn on **Start at login** in the settings window (`/Applications/iris.app/Contents/MacOS/iris --settings`). **Save** writes the LaunchAgent `~/Library/LaunchAgents/dev.iris.app.plist`, and turning it off deletes it ([Start at Login](configuration.md#start-at-login)).

To uninstall, turn off **Start at login**, quit iris (`/Applications/iris.app/Contents/MacOS/iris --quit`), and delete `/Applications/iris.app`.

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

The AppImage holds the iris executable and uses the system's libraries: PipeWire (`libpipewire-0.3`), xkbcommon and xkbcommon-x11, XCB, fontconfig, the Wayland client library, the Vulkan loader, EGL, and GLES 2, the libraries the deb depends on. It mounts itself with FUSE through `fusermount3`, from the `fuse3` package. Without FUSE, run it with `--appimage-extract-and-run`.

Set execution permissions and run:

```sh
chmod +x iris-<ver>-linux-x86_64.AppImage
./iris-<ver>-linux-x86_64.AppImage
```

To start the AppImage at login, turn on **Start at login** in the settings window. **Save** writes `~/.config/autostart/iris-autostart.desktop` running the AppImage's path with `--daemon` ([Start at Login](configuration.md#start-at-login)). Turn it off before moving or deleting the AppImage.

## From Source

[Building](building.md) lists the build dependencies per platform and the package scripts.

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

Before it stops the daemon, `--update` checks the download against the `.sha256` sidecar the release publishes beside the asset. A download whose SHA-256 differs from the sidecar, a body cut short, and a release with no sidecar for the asset each fail the update: `iris --update` deletes the download, installs nothing, leaves the daemon running, prints the error, and exits with status 1. A mismatch prints `update: the downloaded <asset> has SHA-256 <digest>, its .sha256 sidecar lists <digest>; nothing was installed`. `--check-update` reports a newer release with no sidecar for the asset as an error.

Downloads are kept in the `update` directory under the iris cache directory ([paths](configuration.md#iris_home-directory-override)). When a file of the asset's name is already there and its SHA-256 matches the sidecar, `--update` installs that file and downloads nothing. A file there that does not match is replaced by the download.

When the swap fails after `--update` stopped the daemon, it starts the installed iris again as the daemon and appends `; the iris already installed runs again` to the error. When that start fails, it appends `; the iris already installed did not start again, see <log file>`. A daemon that was not running before the update is not started. `iris --update` writes its error to standard error and to the log file ([paths](configuration.md#iris_home-directory-override)).

Platform update mechanisms:
- Windows (installed): An `iris.exe` with `uninstall.exe` beside it downloads `windows-x86_64-setup.exe`, starts it with `/S /RUN` as a detached process that receives no handle of the `iris --update` process, and exits. The installer waits until no process runs the installed `iris.exe`, replaces it, and starts iris. When the installation fails, it starts the `iris.exe` already in place.
- Windows (portable): Any other `iris.exe` downloads `windows-x86_64-portable.zip` and unpacks it with the `tar.exe` Windows ships into `.iris-update` beside `iris.exe`. It renames the running `iris.exe` to `iris.exe.old`, moves the new `iris.exe` into its place, deletes `.iris-update`, and starts the new `iris.exe`. The daemon deletes `iris.exe.old` when it starts. A failed unpack or move deletes `.iris-update` and leaves `iris.exe` as it was. When iris cannot create `.iris-update`, `iris --update` prints `update: cannot write <folder>\.iris-update: <error>; move the portable iris to a folder this account can write, or install it with the setup` and exits with status 1 before downloading anything.
- macOS: Downloads `macos-universal.dmg` and attaches it at a mount point of its own with `hdiutil attach -nobrowse -readonly`. An attach that fails with `Resource temporarily unavailable`, which `hdiutil` reports while another disk image operation runs, is tried up to 30 times, a second apart. It copies the disk image's `iris.app` with `ditto` to `.iris.app.update` beside the bundle iris runs from, detaches the disk image, and swaps the two bundles in one `renamex_np(RENAME_SWAP)` rename, then deletes the old bundle and relaunches the new one. A failure before the swap leaves the installed bundle as it was. An iris outside an `.app` bundle prints `update: <path> is not inside an iris.app bundle; install the release DMG by hand` and exits with status 1 before downloading anything.
- Linux (AppImage): iris runs from an AppImage when the running binary is inside `$APPDIR`, the mount the AppImage runtime sets it to; `$APPIMAGE` then names the AppImage. A program started from another AppImage, such as a terminal, passes that AppImage's `$APPIMAGE` and `$APPDIR` on to the iris it starts, and that iris does not update the other AppImage. The update copies the download beside the AppImage, to its name with the extension `.new` (`iris.AppImage` stages `iris.new`), renames the copy over the AppImage, and launches the new file. A failed copy or rename deletes the copy and leaves the AppImage as it was. When the AppImage's directory takes no new file, `iris --update` prints `update: cannot write <dir>/<stem>.new: <error>; move the AppImage to a folder this account can write, or install the deb or rpm` and exits with status 1 before downloading anything.
- Linux (deb/rpm): An `/usr/bin/iris` that dpkg lists in the `iris` deb downloads `linux-x86_64.deb` and runs `apt-get install -y <file>`. One the `iris` rpm owns downloads `linux-x86_64.rpm` and runs `dnf install -y <file>`. Run as root, `iris --update` runs the command itself. Run as another user, it runs the command through `pkexec`, which prompts for an administrator's password through the session's polkit agent, or on the terminal when no agent runs. The deb and the rpm recommend `pkexec`. When `apt-get` or `dnf` is missing, or `pkexec` is missing for a user other than root, `iris --update` prints the missing program and exits with status 1 before downloading anything. An install that fails prints the command, the reason, and the command that installs the download by hand, for example `update: /usr/bin/pkexec /usr/bin/apt-get install -y <file> was not authorized: the password dialog was dismissed; install it with: sudo apt install <file>`, and the stopped daemon starts again. A successful install starts `/usr/bin/iris` as the daemon. A daemon running while `apt` or `dnf` upgrades the package by hand runs the previous version until it restarts. A **Start at login** entry its settings window writes runs the upgraded `/usr/bin/iris`.
- Linux (other): An `iris` that is neither an AppImage nor the `/usr/bin/iris` of the deb or the rpm, such as one built from source, prints `update: <path> is not from an AppImage, deb or rpm; install the release by hand` and exits with status 1 before downloading anything. The running daemon keeps running.

The [settings window](configuration.md#settings-window) (`iris --settings`) includes a **Check for updates** button in the Updates section that queries releases asynchronously and reports status. When the check finds a newer release, an **Install update** button appears to its left. **Install update** hides both buttons, downloads the asset and checks it against its sidecar off the UI thread, showing `downloading <version>…`, then starts `iris --update` as a detached process and shows `installing <version>; iris restarts when it is done`. That process installs the verified download as described above. When the download fails, the error shows in the status line and both buttons return.
