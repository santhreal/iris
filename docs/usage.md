# Usage

## Start iris

Open iris from the application menu on Linux, the Start menu on Windows, or Applications on macOS, or run `iris`. iris starts, opens its library window, and adds a tray icon. Closing the window leaves iris running in the tray with its global hotkeys. Opening iris again while it runs opens the library window of the running iris; no second iris starts ([Process Model](cli.md#process-model-and-interprocess-communication)).

`iris --background`, which the login autostart entries run, starts iris with no window.

The library window shows past captures ([Capture Library](library.md)). Its toolbar ends with **Settings**, **Record Window** (**Record Screen** on Windows and macOS, which record the desktop), and **New Screenshot**; pointing at a button shows its name and global shortcut. With no captures, the library shows the capture shortcut and the **Record Window** and **New Screenshot** buttons. **New Screenshot** and **Record Window** close the library before the capture or the window pick starts, so the library is not in the picture. While a recording runs, **Record Window** stops it and the library stays open.

The tray menu holds the same actions:

| Item | Action |
| --- | --- |
| **New Screenshot** | Open the region capture overlay |
| **Record Window** (Linux), **Record Screen** (Windows and macOS) | Start a window recording on Linux and a desktop recording on Windows and macOS; shown while no recording runs |
| **Stop Recording** | Stop the recording in progress; shown in place of **Record Window** or **Record Screen** while a recording runs |
| **Library** | Open the capture library |
| **Settings** | Open the settings window |
| **Install iris** *version* | Download, verify, and install the update the last check found; shown only while one is available ([Updates](updates.md)) |
| **Quit** | Quit iris |

A primary or secondary click on the tray icon opens the tray menu.

To start iris at login, turn on **Start iris at login** in the settings window ([Start at Login](configuration.md#start-at-login)). The Windows installer and the deb and rpm packages turn it on at install.

### First start per platform

- **Linux (X11)**: iris grabs the capture and record hotkeys itself. A key another program grabbed first is not bound. The **Shortcuts** pane of the settings window shows **In use by another app** for it; set another key there.
- **Linux (Wayland)**: iris asks the desktop for the capture and record hotkeys through the GlobalShortcuts portal (`org.freedesktop.portal.GlobalShortcuts`). The desktop can show a dialog to confirm them on first use. A desktop with no GlobalShortcuts portal binds none; bind `iris --capture` and `iris --record-window` in the compositor's configuration instead ([Window Manager and Compositor Keybindings](cli.md#window-manager-and-compositor-keybindings)).
- **macOS**: The first capture requests Screen Recording permission. Enable iris in System Settings > Privacy & Security > Screen Recording, then restart iris. The capture hotkey is `Cmd+Shift+2` and the record hotkey `Cmd+Option+Shift+2`: `Cmd+Shift+3` through `Cmd+Shift+6` are the system's screenshot shortcuts, and Mac keyboards have no `Print` key. Menus and tooltips show shortcuts as macOS symbols (`⇧⌘2`).
- **Windows**: The tray icon is in the notification area, or in its overflow menu.

## Take a Screenshot

1. Press `Ctrl+Print` (`Cmd+Shift+2` on macOS), click **New Screenshot** in the library or the tray menu, or run `iris --capture`. The screen freezes and dims.
2. Click a window to capture it (X11, Windows, and macOS), or drag a rectangle and press `Enter` or click **Capture** on the bar at the bottom of the screen. The bar also captures the entire screen or records the screen or a selection. `Escape`, **Cancel**, or a right click cancels.
3. The capture is saved as a PNG in `~/Pictures/iris` and copied to the clipboard.

While the rectangle is drawn, the loupe shows the pixels under the pointer and `c` copies the color at its center. Holding `Space` moves the rectangle instead of resizing it. The arrow keys move the selection by one pixel, ten with `Shift` ([Region Capture](capture.md#region-capture)).

`iris --capture-fullscreen` captures every display, `iris --capture-window` the focused window, and `iris --delay <secs>` every display after a wait ([Capture Modes](capture.md#capture-modes)).

## After a Capture

A thumbnail card, the toast, slides in at a corner of the screen ([Toast Notifications](toast.md)).

- Click it to open the annotation editor.
- Drag it into another application to drop the PNG file there.
- Swipe it away to dismiss it. It also dismisses itself after its countdown.
- Point at it to show its actions: keep it open, copy the image or the file, show the file in its folder, open the editor, or move the capture to the trash.
- Right-click it to copy the text in the image (OCR; on Linux, with Tesseract installed) or to pin the capture in a window of its own.

The editor draws lines, arrows, rectangles, ellipses, pen and highlighter strokes, text, blur, and numbered badges, and crops. **Done** (`Ctrl+S`, `Cmd+S` on macOS) writes the image back to its file and copies it to the clipboard. `Escape` pressed twice discards the changes ([Annotation Editor](editor.md)).

## Record the Screen

1. Press `Ctrl+Shift+Print` (`Cmd+Option+Shift+2` on macOS), click **Record Window** (**Record Screen** on Windows and macOS) in the library or the tray menu, or run `iris --record-window`.
2. On X11, click the window to record. A card in the toast corner reads **Click a window to record it** until the click, and `Escape` cancels. A click on a menu, a tooltip, or the card itself picks nothing. On Wayland, pick the window in the compositor's dialog. Windows and macOS record the whole desktop.
3. A chip with a timer and pause, stop, and microphone buttons floats beside the recorded area (X11, Windows, and macOS). A GIF recording has no microphone button.
4. Press the record hotkey again, click the chip's stop button, or run `iris --record-window` to stop.

`iris --record-region` records a rectangle drawn as for a screenshot (X11, Windows, and macOS). Recordings are saved in `~/Videos/iris`. Release packages include FFmpeg; source builds use `ffmpeg` from `PATH` ([Screen Recording](recording.md)).

## Find Past Captures

Open iris, click **Library** in the tray menu, or run `iris --library`. The library shows the captures as thumbnails, newest first. Click a thumbnail to open it in the editor, drag thumbnails out to drop their files, and select several to copy or delete them at once ([Capture Library](library.md)).

## Change Settings

Click the gear in the library's toolbar, click **Settings** in the tray menu, or run `iris --settings`. The settings window has five tabs: **General** (Start at login, save folders, file names), **Capture** (capture feedback and the floating thumbnail), **Recording**, **Shortcuts**, and **Updates**. Each change writes `config.toml` at once, and a changed hotkey registers again in the running iris at once ([Settings Window](configuration.md#settings-window)).

## Quit

Click **Quit** in the tray menu, or run `iris --quit`. A recording in progress is saved before iris exits.
