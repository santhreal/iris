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
- **Library**: the main window lists captures with thumbnails; copy,
  annotate, reveal, or delete from there.
- **CLI flags forward to the running instance**, so compositor keybinds
  (Hyprland, i3, sxhkd) drive the same iris process.

## Install

Requires Rust and ffmpeg on `PATH`.

```sh
cargo build --release
```

The binary is `iris`. Run it once to start the daemon (tray, global
hotkeys); later invocations forward flags to the running instance.

For development: `cargo run`.

## Configuration

`~/.config/iris/config.toml` (created on first run; an existing
`~/.config/glint/config.toml` is migrated automatically):

```toml
screenshots_dir = "~/Pictures/iris"
recordings_dir = "~/Videos/iris"
screenshot_template = "{date}_{time}"
record_mic_default = false
recording_fps = 30
show_toast_after_capture = true
```

A leading `~` expands to your home directory.

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
