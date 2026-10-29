# Changelog

Version numbers follow [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html). Entries follow [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- iris runs on Linux (X11 and Wayland), Windows, and macOS as a background daemon that holds the global hotkeys and a tray menu.
- `iris --capture` and the other command-line options forward to the running daemon over a local socket or named pipe, so a window manager or compositor keybinding runs them.
- A region capture freezes the screen and opens a selection overlay with window snapping, an 8x loupe, and a pixel color sampler.
- `--capture-fullscreen`, `--capture-window`, and `--delay <secs>` capture the whole display, the focused window, or the display after a delay.
- A delayed capture counts down on a corner card whose close button cancels it, and the card leaves the screen before the capture.
- A capture is saved as a PNG, copied to the clipboard, and shown in a thumbnail toast.
- The toast drags out as a file, copies the image or the file, shows the file in its folder, opens the editor, pins the capture in an always-on-top window, and copies the text in the image through OCR (Vision on macOS, Tesseract on Linux and Windows).
- The annotation editor draws lines, arrows, rectangles, ellipses, pen and highlighter strokes, text, blur, and numbered badges, and crops.
- Undo and redo in the editor cover crops, rotations, flips, and Clear Markup, and undoing a crop or transformation restores the previous image with its annotations editable.
- `Ctrl+V` or `Cmd+V` in the editor's text entry and in a Settings text field appends the clipboard's text as one line.
- `Ctrl+Backspace` or `Option+Backspace` in the editor's text entry and in a Settings text field deletes the last word, and `Cmd+Backspace` deletes the whole text.
- `Up`, `Down`, `Return`, and `Space` move through and choose from an open menu: a Settings pop-up, the editor's Copy and More menus, and the Library's context menu.
- Window and region recording through ffmpeg writes MP4, GIF, or WebM, with pause, a microphone toggle, and an indicator chip that is excluded from the recording where the platform allows it.
- The tray menu shows Stop recording in place of Record window while a recording runs.
- A touchpad pinch zooms the editor's image about the pinch center.
- The capture library shows every capture in a thumbnail grid with multiple selection, file drag-out, copy, and reveal in the file manager.
- The library has keyboard navigation, a context menu on each card, and a Quick Look preview on `Space`.
- The library's card context menu pins a capture to the screen.
- Moving a capture to the trash from the library or the toast moves the file to the freedesktop Trash on Linux, the Trash on macOS, or the Recycle Bin on Windows, where the file manager can restore it.
- Every window sets its text in the bundled Inter typeface, and the editor's text tool draws in Inter Bold.
- The home window shows the app icon, the running version, and New Screenshot, Record Window, and Library tiles captioned with their shortcuts.
- The home, settings, library, and editor windows share one toolbar control style.
- The settings window groups its options in General, Capture, Recording, Shortcuts, and Updates panes selected from its toolbar.
- The settings window writes each change to `config.toml` as it is made: a toggle or menu choice on the click, a text field on `Return` or when it loses focus, and a shortcut when it is pressed.
- The settings window's **Start iris at login** toggle reads and writes the account's XDG autostart entry, LaunchAgent, or Run registry value.
- Releases ship a Windows installer and portable zip for x86_64 and aarch64, a universal macOS disk image, and deb, rpm, and AppImage packages for Linux x86_64 and aarch64.
- Every release package bundles FFmpeg with its license notices and source locations, so recording needs no separate FFmpeg installation.
- Every release asset has a `.sha256` checksum sidecar and a minisign `.minisig` signature made with the key in `packaging/minisign.pub`.
- `iris --update` installs the latest release in place for the Windows installer, the portable zip, the macOS app, the AppImage, the deb, and the rpm after it checks the download's SHA-256 and minisign signature.
- The daemon checks for a newer release about a minute after it starts and then once a day, and offers it in the tray menu, a notice, and the settings window's Updates pane.
- `check_for_updates = false` in `config.toml` turns off the background update check.
- `update_channel = "beta"` in `config.toml` offers prereleases as well as stable releases.
- On Wayland the daemon binds the capture and record hotkeys through the GlobalShortcuts portal where the desktop provides it.
- The home window and the settings window's Shortcuts pane show a global hotkey that another program holds as unavailable, with the reason.
- An X11 window pick shows a card that states the click and the `Escape` key until the pick ends.
- The AppImage adds its application menu entry and icon on each start, and turns on Start at login on its first start.
- The macOS app turns on Start at login on its first start outside the disk image.
- A panic on any thread writes its thread, source location, and message to `iris.log`.

### Changed

- Reduced capture-overlay, capture-toast, Library-opening, and wheel-scrolling latency.
- A bare `iris` opens the home window; `iris --daemon` starts the daemon with no window.
- The default capture hotkey is `Ctrl+Print` and the default record hotkey is `Ctrl+Shift+Print`.
- The home window shows each shortcut as key caps, in a larger layout with a hairline window edge.
- The daemon runs the command it started for, such as opening the home window, before it prepares the capture renderer, and skips that preparation while a recording runs.
- An empty library shows no `0 captures` count, and its hint names the capture shortcut only while that shortcut is bound.

### Fixed

- Library hover animations no longer skip after an idle window.
- Concurrent captures and recordings reserve distinct output files instead of overwriting a file selected for the same timestamp.
- A failed portable Windows update preserves and reports recovery files instead of deleting them during cleanup or a retry.
- The New Screenshot and Record Window tiles close the home window, and the library's New Screenshot button closes the library, before the capture starts, so neither appears in the frozen frame.
- The capture overlay opens with the window under a resting pointer snapped and the pointer readout at the pointer, instead of waiting for the pointer to move.
- An X11 window pick ignores a click on a menu, a tooltip, or an iris card.
- An icon in a row whose text overflows keeps its size instead of shrinking to a sliver, as the folder glyph in the settings window's storage rows did.

### Security

- Updates `rustls` to 0.23.46, `rustls-webpki` to 0.103.16, and `webpki-roots` to 1.0.10 to resolve six security advisories.
