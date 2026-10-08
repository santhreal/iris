# Changelog

Version numbers follow [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html). Entries follow [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- iris runs on Linux (X11 and Wayland), Windows, and macOS as a background daemon that holds the global hotkeys and a tray menu.
- `iris --capture` and the other command-line options forward to the running daemon over a local socket or named pipe, so a window manager or compositor keybinding runs them.
- A region capture freezes the screen and opens a selection overlay with window snapping, an 8x loupe, and a pixel color sampler.
- `--capture-fullscreen`, `--capture-window`, and `--delay <secs>` capture the whole display, the focused window, or the display after a delay.
- A capture is saved as a PNG, copied to the clipboard, and shown in a thumbnail toast.
- The toast drags out as a file, copies the image or the file, shows the file in its folder, opens the editor, pins the capture in an always-on-top window, and copies the text in the image through Tesseract OCR.
- The annotation editor draws lines, arrows, rectangles, ellipses, pen and highlighter strokes, text, blur, and numbered badges, and crops.
- Window and region recording through ffmpeg writes MP4, GIF, or WebM, with pause, a microphone toggle, and an indicator chip that is excluded from the recording where the platform allows it.
- The capture library shows every capture in a thumbnail grid with multiple selection, file drag-out, copy, and reveal in the file manager.
- The library has keyboard navigation, a context menu on each card, and a Quick Look preview on `Space`.
- Moving a capture to the trash from the library or the toast moves the file to the freedesktop Trash on Linux, the Trash on macOS, or the Recycle Bin on Windows, where the file manager can restore it.
- Every window sets its text in the bundled Inter typeface, and the editor's text tool draws in Inter Bold.
- The home window shows the app icon, the running version, and New Screenshot, Record Window, and Library tiles captioned with their shortcuts.
- The home, settings, library, and editor windows share one toolbar control style.
- The settings window groups its options in General, Capture, Recording, Shortcuts, and Updates panes selected from its toolbar.
- The settings window writes each change to `config.toml` as it is made: a toggle or menu choice on the click, a text field on `Return` or when it loses focus, and a shortcut when it is pressed.
- The settings window's **Start iris at login** toggle reads and writes the account's XDG autostart entry, LaunchAgent, or Run registry value.
- Releases ship a Windows installer and portable zip for x86_64 and aarch64, a universal macOS disk image, and deb, rpm, and AppImage packages for Linux x86_64 and aarch64.
- Every release asset has a `.sha256` checksum sidecar and a minisign `.minisig` signature made with the key in `packaging/minisign.pub`.
- `iris --update` installs the latest release in place for the Windows installer, the portable zip, the macOS app, the AppImage, the deb, and the rpm after it checks the download's SHA-256 and minisign signature.
- The daemon checks for a newer release about a minute after it starts and then once a day, and offers it in the tray menu, a notice, and the settings window's Updates pane.
- `check_for_updates = false` in `config.toml` turns off the background update check.
- `update_channel = "beta"` in `config.toml` offers prereleases as well as stable releases.
