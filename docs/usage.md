# Usage

## Start iris

Run `iris`. The daemon starts in the background and adds a tray icon. It holds the global hotkeys and runs every command. A second `iris` forwards its command to the running daemon and exits ([Process Model](cli.md#process-model-and-interprocess-communication)).

The application menu entry on Linux and the Start menu shortcut on Windows run `iris --home`, which opens the home window. On macOS, opening iris.app while the daemon runs with no iris window open shows the home window.

The home window has three tiles, **New Screenshot**, **Record Window**, and **Library**, and a gear that opens the settings window. `Escape` closes it.

The tray menu holds the same actions. A primary or secondary click on the tray icon opens it:

| Item | Action |
| --- | --- |
| **Capture** | Open the region capture overlay |
| **Record window** | Start a window recording, or stop the one in progress |
| **Library** | Open the capture library |
| **Settings** | Open the settings window |
| **Quit** | Stop the daemon |

To start the daemon at login, turn on **Start at login** in the settings window ([Start at Login](configuration.md#start-at-login)). The Windows installer and the deb and rpm packages turn it on at install.

### First start per platform

- **Linux (X11)**: iris grabs the capture and record hotkeys itself.
- **Linux (Wayland)**: Wayland gives no client a global hotkey. Bind `iris --capture` and `iris --record-window` in the compositor's configuration ([Window Manager and Compositor Keybindings](cli.md#window-manager-and-compositor-keybindings)).
- **macOS**: The first capture requests Screen Recording permission. Enable iris in System Settings > Privacy & Security > Screen Recording, then restart iris. The capture hotkey is `F13`: macOS keyboards have no `Print` key.
- **Windows**: The tray icon is in the notification area, or in its overflow menu.

## Take a Screenshot

1. Press `Print` (`F13` on macOS), click **New Screenshot**, click **Capture** in the tray menu, or run `iris --capture`. The screen freezes and dims.
2. Click a window to capture it (X11, Windows, and macOS), or drag a rectangle and press `Enter`. `Escape` or a right click cancels.
3. The capture is saved as a PNG in `~/Pictures/iris` and copied to the clipboard.

While the rectangle is drawn, the loupe shows the pixels under the pointer and `c` copies the color at its center. The arrow keys move the selection by one pixel, ten with `Shift` ([Region Capture](capture.md#region-capture)).

`iris --capture-fullscreen` captures every display, `iris --capture-window` the focused window, and `iris --delay <secs>` every display after a wait ([Capture Modes](capture.md#capture-modes)).

## After a Capture

A thumbnail card, the toast, slides in at a corner of the screen ([Toast Notifications](toast.md)).

- Click it to open the annotation editor.
- Drag it into another application to drop the PNG file there.
- Swipe it away to dismiss it. It also dismisses itself after its countdown.
- Its action bar pins it, copies the image or the file, shows the file in its folder, opens the editor, or deletes the capture.
- Right-click it to copy the text in the image (OCR, with Tesseract installed) or to pin the capture in a window of its own.

The editor draws lines, arrows, rectangles, ellipses, pen and highlighter strokes, text, blur, and numbered badges, and crops. **Done** (`Ctrl+S`, `Cmd+S` on macOS) writes the image back to its file and copies it to the clipboard. `Escape` discards the changes ([Annotation Editor](editor.md)).

## Record the Screen

1. Press `Ctrl+Shift+R`, click **Record Window**, click **Record window** in the tray menu, or run `iris --record-window`.
2. On X11, click the window to record. On Wayland, pick it in the compositor's dialog. Windows and macOS record the whole desktop.
3. A chip with a timer and pause, stop, and microphone buttons floats beside the recorded area (X11, Windows, and macOS). A GIF recording has no microphone button.
4. Press `Ctrl+Shift+R` again, click the chip's stop button, or run `iris --record-window` to stop.

`iris --record-region` records a rectangle drawn as for a screenshot (X11, Windows, and macOS). Recordings are saved in `~/Videos/iris`. Recording requires `ffmpeg` ([Screen Recording](recording.md)).

## Find Past Captures

Click **Library**, or run `iris --library`. The library shows the captures as thumbnails, newest first. Click a thumbnail to open it in the editor, drag thumbnails out to drop their files, and select several to copy or delete them at once ([Capture Library](library.md)).

## Change Settings

Click the gear in the home window, click **Settings** in the tray menu, or run `iris --settings`. The settings window edits the save folders and file names, the capture and toast behavior, recording, the hotkeys, and Start at login, and checks for updates. **Save** writes `config.toml` and registers the hotkeys again in the running daemon ([Configuration](configuration.md)).

## Quit

Click **Quit** in the tray menu, or run `iris --quit`. A recording in progress is saved before the daemon exits.
