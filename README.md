# iris

Screenshot and screen-recording utility. Press a key, click a window or
drag a region, done: the capture is on disk and on the clipboard before
you look away.

## What it does

- **Capture** (`Print` or `iris --capture`): freezes the screen, dims it,
  and lets you click a window (hover shows a dashed snap outline) or drag
  a region. A live loupe shows 8x zoom with pixel coordinates and hex
  color. The PNG saves to your screenshots folder and copies to the
  clipboard immediately. A corner toast offers annotate / open / delete;
  the annotation editor is opt-in, never in the way.
- **Record window** (`Ctrl+Shift+R` or `iris --record-window`): crosshair
  window pick on X11, portal source pick on Wayland. While recording, a
  click-through red border wraps the target window and a floating chip
  shows the elapsed time and mic state. Stop with the same hotkey, the
  tray, or `iris --stop-recording`. Output is H.264/AAC mp4 via ffmpeg.
- **Record region** (`iris --record-region`): drag a screen region and
  record just that area.
- **Pin to screen**: pin a capture as an always-on-top reference image;
  drag to move, double-click or Esc to close.
- **Annotate**: the editor has pen, line, arrow, rect, ellipse, text,
  highlight, blur, crop, and a numbered-counter tool; undo/redo, fill
  toggle, zoom and pan. Press `?` in the editor for the shortcut sheet.
- **OCR**: the toast's context menu can copy recognized text from a
  capture.
- **Library**: the main window lists captures with thumbnails; copy,
  annotate, reveal, or delete from there.
- **CLI flags forward to the running instance**, so compositor keybinds
  (Hyprland, i3, sxhkd) drive the same iris process.

## Install

Download an installer from the
[releases](https://github.com/santhreal/iris/releases) page. Each asset
has a `.sha256` checksum beside it.

### Windows

Run `iris-<ver>-windows-x86_64-setup.exe`. The installer is per-user
(no admin), puts `iris.exe` in `%LOCALAPPDATA%\Programs\iris`, adds a
Start Menu shortcut, and registers iris to start at login. Uninstall
from Add/Remove Programs.

### macOS

Open `iris-<ver>-macos-universal.dmg` and drag `iris.app` to
Applications. iris is a menu-bar app (`LSUIElement`), so it does not
appear in the Dock; control it from the status item. To start it at
login, install the LaunchAgent:

```sh
mkdir -p ~/Library/LaunchAgents
cp /Applications/iris.app/Contents/Resources/dev.iris.app.plist ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/dev.iris.app.plist
```

### Linux

Debian/Ubuntu: `sudo apt install ./iris-<ver>-linux-x86_64.deb`.
Fedora/RHEL: `sudo dnf install ./iris-<ver>-linux-x86_64.rpm`.
Anywhere else: make `iris-<ver>-linux-x86_64.AppImage` executable and
run it. The deb/rpm install a `.desktop` launcher and an XDG autostart
entry; the AppImage is self-contained.

### From source

Requires Rust and ffmpeg on `PATH`.

```sh
cargo build --release
```

The binary is `iris`. Run it once to start the daemon (tray, global
hotkeys); later invocations forward flags to the running instance.

For development: `cargo run`.

## Updates

`iris --check-update` reports whether a newer release exists.
`iris --update` downloads the platform asset and applies it: the NSIS
installer on Windows, a `.app` swap on macOS, an in-place AppImage
replace on Linux. A deb/rpm install updates through the package
manager instead. The Settings window has a Check-for-updates button
that reports through the status pill.

## Configuration

`~/.config/iris/config.toml` (created on first run; an existing
`~/.config/glint/config.toml` is migrated automatically):

```toml
screenshots_dir = "~/Pictures/iris"
recordings_dir = "~/Videos/iris"
screenshot_template = "{date}_{time}"
record_mic_default = false
recording_fps = 30
recording_format = "mp4"        # mp4 | gif | webm
recording_encoder = "auto"      # auto | libx264 | nvenc
show_toast_after_capture = true
```

A leading `~` expands to your home directory. The Settings window
(`iris --settings`) edits every field.

## CLI

Every flag forwards to the running daemon, so compositor keybinds drive
one process.

| Flag | Action |
| --- | --- |
| `iris --capture` | region capture overlay, then toast |
| `iris --capture-fullscreen` | full-screen grab, no overlay |
| `iris --capture-window` | capture the focused window |
| `iris --delay <secs>` | full-screen capture after a countdown |
| `iris --record-window` | toggle a window-picked recording |
| `iris --record-region` | pick a screen region and record it |
| `iris --record-pause` | pause/resume the active recording |
| `iris --record-mic` | toggle the mic on the active recording |
| `iris --library` | open the library |
| `iris --settings` | open the settings window |
| `iris --annotate <file>` | edit an existing capture |
| `iris --quit` | stop the daemon |
| `iris --version` | print the version and exit |
| `iris --check-update` | report whether a newer release exists |
| `iris --update` | download and apply the latest release |

## Keys

| Key | Action |
| --- | --- |
| `Print` | capture overlay |
| `Ctrl+Shift+R` | start/stop recording |
| `Esc` | cancel overlay or window pick |
| `Enter` | confirm selection / save annotation |

## Platform support

| Platform | Capture | Recording |
| --- | --- | --- |
| Linux/X11 | full support, window hover-snap | per-window, border + chip |
| Linux/Wayland | xdg-desktop-portal | portal ScreenCast + PipeWire |
| Windows | ffmpeg gdigrab | desktop record (gdigrab) |
| macOS | ffmpeg avfoundation | desktop record (avfoundation) |

Wayland recording asks the portal for a window source; the compositor's
own picker appears. Windows and macOS record the primary desktop and
stop via hotkey, tray, or `--stop-recording`.

Single-instance IPC, global hotkeys, and the tray run on all three
platforms: a local socket on Linux, a named pipe on Windows, and a
local socket on macOS; X11 key grabs, `RegisterHotKey`, and Carbon
hotkeys; StatusNotifierItem, `Shell_NotifyIcon`, and `NSStatusItem`.
