# Platform Backends

iris implements native platform backends across Linux (X11 and Wayland), Windows, and macOS.

## Platform Capabilities Matrix

| Subsystem | Linux (X11) | Linux (Wayland) | Windows | macOS |
| --- | --- | --- | --- | --- |
| **Region Capture** | Root window grab via MIT-SHM (`x11rb`) | Frozen-frame overlay over portal capture | Win32 GDI `BitBlt` (`CAPTUREBLT`) | CoreGraphics `CGWindowListCreateImage` |
| **Window Hover Snap** | Supported (`_NET_CLIENT_LIST_STACKING`) | Unsupported (compositor security blocks tree) | Supported (`EnumWindows`) | Supported (`CGWindowListCopyWindowInfo`) |
| **Active-Window Capture** | Supported (`_NET_ACTIVE_WINDOW`) | Unsupported (compositor security blocks query) | Supported (`GetForegroundWindow`) | Supported (Frontmost normal-layer window) |
| **Fullscreen Capture** | Supported (Root window via MIT-SHM) | Supported (`org.freedesktop.portal.Screenshot`) | Supported (Virtual screen union via GDI `BitBlt`) | Supported (Display union `CGWindowListCreateImage`) |
| **Window Recording** | Supported (XComposite redirect + MIT-SHM) | Supported (`org.freedesktop.portal.ScreenCast` + PipeWire) | Unsupported (Records primary desktop via `gdigrab`) | Unsupported (Records main desktop via `avfoundation`) |
| **Region Recording** | Supported (Root window via MIT-SHM) | Unsupported (Portal cannot name rect) | Supported (`ffmpeg -f gdigrab`) | Supported (`ffmpeg -f avfoundation -vf crop`) |
| **Recording Chip** | Supported (Positioned by XID; outside frame) | Disabled | Supported (`SetWindowDisplayAffinity`) | Supported (`NSWindowSharingNone`) |
| **Global Hotkeys** | X11 key grabs (`xcb_grab_key`) | Compositor keybindings via IPC | Win32 `RegisterHotKey` | Carbon `RegisterEventHotKey` |
| **System Tray** | StatusNotifierItem (`ksni`) | StatusNotifierItem (`ksni`) | Win32 `Shell_NotifyIconW` | AppKit `NSStatusItem` |
| **Single-Instance IPC** | Unix socket (`$XDG_RUNTIME_DIR/iris.sock`) | Unix socket (`$XDG_RUNTIME_DIR/iris.sock`) | Named pipe (`\\.\pipe\iris`) | Unix socket (`$XDG_RUNTIME_DIR/iris.sock` or `$TMPDIR/iris.sock`) |
| **Clipboard** | `arboard` (images); X11 window (`text/uri-list`) | `arboard` (images); file list unsupported | `arboard` (images); `CF_HDROP` (files) | `arboard` (images); `NSPasteboard` `NSURL` (files) |
| **File Drag-Out** | XDND protocol via private window | Unsupported | Win32 `SHDoDragDrop` (OLE) | AppKit `NSDraggingSession` |
| **Reveal in Folder** | DBus `org.freedesktop.FileManager1.ShowItems` | DBus `org.freedesktop.FileManager1.ShowItems` | Win32 `SHOpenFolderAndSelectItems` | Command `/usr/bin/open -R` |
| **Shutter Sound** | `pw-play` / `paplay` / `canberra-gtk-play` | `pw-play` / `paplay` / `canberra-gtk-play` | Win32 `PlaySoundW` (synthesized WAV) | Command `/usr/bin/afplay` |
| **Window Move and Resize** | Title bar and edge strips; `_NET_WM_MOVERESIZE`, or the window follows the pointer on XInput 2.1 raw events | Title bar and edge strips; `xdg_toplevel.move` / `xdg_toplevel.resize` | `WM_NCLBUTTONDOWN(HTCAPTION)` move; frame hit test (`WM_NCHITTEST`) resize | `performWindowDragWithEvent:` move; AppKit frame resize |
| **Window Activation** | `_NET_ACTIVE_WINDOW` request and `SetInputFocus` | `xdg_activation_v1` token; the compositor's focus policy applies | `ShowWindowAsync(SW_RESTORE)` when minimized, then `SetForegroundWindow` | `makeKeyAndOrderFront:` |
| **Update / Install** | AppImage overwrite via `$APPIMAGE` rename | AppImage overwrite via `$APPIMAGE` rename | Detached NSIS installer (`/S` silent) | DMG mount via `hdiutil`, replace `.app` |

## Linux (X11)

- **Screen Capture**: Uses the X11 Shared Memory Extension (MIT-SHM) via `x11rb` to read frame buffers from the root window, falling back to `GetImage`.
- **Window Hierarchy and Snap**: Queries EWMH `_NET_CLIENT_LIST_STACKING`, falling back to root `query_tree`. Reads `_NET_WM_PID` to exclude iris windows and `_NET_WM_STATE` to exclude `_NET_WM_STATE_HIDDEN`. Active-window capture queries `_NET_ACTIVE_WINDOW`.
- **Recording**: Window recording redirects the target window via XComposite (`composite_redirect_window`) and captures its named pixmap via MIT-SHM. Red border strips (`#f7768e`) with empty XFixes input shapes outline the window. A tracking thread follows `ConfigureNotify` events to move the border strips and reposition the floating indicator chip by XID (`configure_window`). Region recording captures root window rectangles via MIT-SHM.
- **Global Hotkeys**: Registers passive root key grabs via `grab_key` (`xcb_grab_key`) across modifier lock combinations.
- **System Tray**: Registers a StatusNotifierItem interface using `ksni`.
- **IPC Transport**: Binds a Unix domain socket at `$XDG_RUNTIME_DIR/iris.sock` (mode 0600, fallback `/tmp/iris.sock`) using `interprocess::local_socket`.
- **Drag-Out and Clipboard**: Drag-out speaks the XDND protocol from a private 1x1 window tracking the pointer via `XQueryPointer`. File-path clipboard operations serve `text/uri-list` on the X11 `CLIPBOARD` selection. Image clipboard operations run through `arboard`.
- **Reveal in Folder**: Calls DBus method `org.freedesktop.FileManager1.ShowItems` via `dbus-send`, falling back to `xdg-open`.
- **Shutter Sound**: Executes `pw-play /usr/share/sounds/freedesktop/stereo/camera-shutter.oga`, falling back to `paplay`, then `canberra-gtk-play -i camera-shutter`.
- **Window Identity**: Every iris window sets `WM_CLASS` to `dev.iris.app` for both the instance and the class. The desktop entry `dev.iris.app.desktop` has the same id and sets `StartupWMClass=dev.iris.app`, and taskbars and docks list iris windows under its name and icon. Titles distinguish the windows: `iris` (home), `Library - iris`, `Settings - iris`, `<file> - iris` (editor), `Capture - iris` (selection overlay), `Screenshot - iris` (toast), `Pin - iris`, `Recording - iris` (indicator chip), `Notice - iris`, and `Flash - iris`.
- **Window Move and Resize**: The library, editor, and settings windows draw their own frame. A title bar drag begins once the pointer is more than 4 pixels from the press; a click leaves the window in place. Resize strips run along the inside of the window border: 6 pixels on each side and 16-pixel squares at the corners. Maximized and fullscreen windows have no strips, and a side tiled against a screen edge or another window has none. When the running window manager lists `_NET_WM_MOVERESIZE` in `_NET_SUPPORTED` and its `_NET_SUPPORTING_WM_CHECK` window names itself, a title bar drag or a press on a strip sends `_NET_WM_MOVERESIZE` from the press position, and the window manager moves or resizes the window and holds the minimum size from `WM_NORMAL_HINTS`. Under any other window manager the window follows the pointer: iris selects XInput 2.1 raw motion and raw button release events on the root window, reads the pointer on each one, and moves or resizes the window with `configure_window` until button 1 is released. A resize keeps the opposite edges in place, stops at the window's minimum size, and changes the size at most once every 16 ms. On a server without XInput 2.1 the pointer is read every 8 ms.
- **Update**: Copies the downloaded AppImage over `$APPIMAGE` using an atomic file rename.

## Linux (Wayland)

- **Screen Capture**: Full-screen frames are acquired via `org.freedesktop.portal.Screenshot` using `ashpd`. The selection overlay opens over the captured frozen frame.
- **Window Hierarchy and Snap**: Unsupported. Wayland compositor protocols isolate clients, preventing foreign window inspection and active-window queries.
- **Recording**: Window recording invokes `org.freedesktop.portal.ScreenCast` via `ashpd` with `SourceType::Window` and `CursorMode::Hidden`. Video frames arrive over PipeWire as `MemPtr`, `MemFd`, or `DMA-buf` (imported through EGL and read with `glReadPixels`, or read through CPU mapping). The segment recorder accepts pause and microphone controls over IPC. Region recording is unsupported. The floating indicator chip is disabled.
- **Global Hotkeys**: Wayland protocols reject client-side global key grabs. Shortcuts are defined in compositor configurations (such as Hyprland or Sway) to execute `iris --capture` or `iris --record-window`. Invocations forward commands to the daemon over the IPC socket.
- **System Tray**: Registers a StatusNotifierItem interface using `ksni`.
- **Window Identity**: Every iris window sets the `xdg_toplevel` app id `dev.iris.app`, the id of the desktop entry `dev.iris.app.desktop`. Titles are the same as on X11. A window rule selects one iris window by title, for example in sway: `for_window [app_id="dev.iris.app" title="^Pin - iris$"] sticky enable`.
- **Window Activation**: A second open of the home, library, settings, or editor window requests an `xdg_activation_v1` token and activates the open window with it. The compositor's focus policy applies to the request: sway, by default (`focus_on_window_activation urgent`), marks the window urgent instead of focusing it.
- **IPC Transport**: Binds a Unix domain socket at `$XDG_RUNTIME_DIR/iris.sock` (mode 0600).
- **Drag-Out and Clipboard**: File drag-out and file-list clipboard copies are unsupported. Image clipboard operations run through `arboard`.
- **Reveal in Folder**: Calls DBus method `org.freedesktop.FileManager1.ShowItems` via `dbus-send`, falling back to `xdg-open`.
- **Shutter Sound**: Executes `pw-play /usr/share/sounds/freedesktop/stereo/camera-shutter.oga`, `paplay`, or `canberra-gtk-play`.
- **Window Move and Resize**: A title bar drag calls `xdg_toplevel.move` with the press's serial once the pointer is more than 4 pixels from the press. The resize strips are the same as on X11; a press on one calls `xdg_toplevel.resize` with its edge, and the compositor enforces the minimum size from `xdg_toplevel.set_min_size`.
- **Update**: Copies the downloaded AppImage over `$APPIMAGE` using an atomic file rename.

## Windows

- **Process**: `iris.exe` is a GUI-subsystem program: no console window opens for the daemon, the login autostart, the Start Menu shortcut, or a hotkey tool running `iris --capture`. A command typed in `cmd.exe` or PowerShell attaches to that terminal's console to print its output and errors. The terminal returns to its prompt without waiting, so the output prints after the prompt. `start /wait iris --version` in `cmd.exe` and `iris --version | Write-Output` in PowerShell wait for the command and set `%ERRORLEVEL%` and `$LASTEXITCODE`.
- **Screen Capture**: Uses Windows GDI APIs (`CreateCompatibleDC`, `BitBlt` with `CAPTUREBLT | SRCCOPY`) into a 32-bit DIB section. Virtual screen bounds are determined via `GetSystemMetrics` (`SM_XVIRTUALSCREEN`, `SM_YVIRTUALSCREEN`, `SM_CXVIRTUALSCREEN`, `SM_CYVIRTUALSCREEN`).
- **Window Hierarchy and Snap**: Enumerates visible top-level windows using `EnumWindows`, `IsWindowVisible`, and `GetWindowRect`. Active-window capture queries `GetForegroundWindow`.
- **Recording**: Desktop recording executes `ffmpeg` with `-f gdigrab -framerate <fps> -draw_mouse 1 -i desktop` (and `-f dshow -i audio=<device>` when microphone capture is active). Region recording supplies `-offset_x <x> -offset_y <y> -video_size <width>x<height>`. Window recording records the desktop. Encodes stretches into Matroska segments (`part<N>.mkv`), joined on stop.
- **Indicator Chip**: Sets `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` on the window handle so GDI, `gdigrab`, and Desktop Window Manager exclude the chip from captures.
- **Global Hotkeys**: Registers system-wide shortcut chords via Win32 `RegisterHotKey` on a dedicated thread pumping `GetMessageW` and handling `WM_HOTKEY`. Settings reload uses `WM_APP + 1`.
- **System Tray**: Creates a notification icon via `Shell_NotifyIconW` associated with an `HWND_MESSAGE` window.
- **IPC Transport**: Binds a Windows named pipe at `\\.\pipe\iris` using `interprocess::local_socket`.
- **Drag-Out and Clipboard**: File drag-out executes on an STA thread. `SHCreateShellItemArrayFromIDLists` and `IShellItemArray::BindToHandler(BHID_DataObject)` build the shell's data object for the files, which holds `CF_HDROP`, the shell ID list array, and file descriptors, and `SHDoDragDrop` (`DROPEFFECT_COPY`) runs the drag. File-path clipboard copy writes `CF_HDROP` payloads (`DROPFILES`) via `OpenClipboard` and `SetClipboardData`. Image clipboard operations run through `arboard`.
- **Reveal in Folder**: Calls Win32 Shell API `SHOpenFolderAndSelectItems` with `ILCreateFromPathW`, falling back to `explorer.exe`.
- **Shutter Sound**: Plays an in-memory synthesized WAV buffer using Win32 `PlaySoundW` (`SND_MEMORY | SND_ASYNC | SND_NODEFAULT | SND_SYSTEM`).
- **Update**: Spawns a detached `cmd.exe` process that executes the NSIS installer `windows-x86_64-setup.exe` in silent mode (`/S`).

## macOS

- **Screen Capture**: Captures display and window buffers via CoreGraphics `CGWindowListCreateImage` over `CGRect` (`kCGWindowListOptionOnScreenOnly`, `kCGNullWindowID`). Permission is checked with `CGPreflightScreenCaptureAccess` and requested with `CGRequestScreenCaptureAccess`.
- **Window Hierarchy and Snap**: Queries on-screen windows via `CGWindowListCopyWindowInfo` (`kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktop`), filtering layer 0 windows and excluding iris processes. Active-window capture selects the frontmost normal-layer window.
- **Recording**: Desktop recording executes `ffmpeg` with `-f avfoundation -framerate <fps> -capture_cursor 1 -i <screen>:<mic|none>`. Region recording applies `-vf crop=<width>:<height>:<x>:<y>`. Window recording records the main display desktop. Encodes stretches into Matroska segments (`part<N>.mkv`), joined on stop.
- **Indicator Chip**: Sets `[ns_window setSharingType: 0]` (`NSWindowSharingNone`) to exclude the window from Quartz and `avfoundation` display captures.
- **Global Hotkeys**: Registers shortcut chords via Carbon `RegisterEventHotKey` and installs an event handler with `InstallEventHandler` for `kEventClassApplication` / `kEventHotKeyPressed` on `GetApplicationEventTarget()`. Does not require Accessibility permissions.
- **System Tray**: Creates an `NSStatusItem` in the macOS menu bar with an `NSMenu`.
- **Reopen**: Opening iris.app while the daemon runs starts no second process. AppKit sends the running daemon `applicationShouldHandleReopen:hasVisibleWindows:`, and with no iris window visible the daemon opens the home window.
- **IPC Transport**: Binds a Unix domain socket under `$XDG_RUNTIME_DIR/iris.sock` or `$TMPDIR/iris.sock` (mode 0600) using `interprocess::local_socket`.
- **Drag-Out and Clipboard**: File drag-out initiates `beginDraggingSessionWithItems:event:source:` on the window content `NSView` using `NSDraggingItem` and `NSDraggingSource` (`NSDragOperationCopy`). File-path clipboard copy writes `NSURL` file objects to `NSPasteboard` (`writeObjects:`). Image clipboard operations run through `arboard`.
- **Reveal in Folder**: Executes `/usr/bin/open -R <path>`, falling back to `/usr/bin/open <folder>`.
- **Shutter Sound**: Plays `/System/Library/Components/CoreAudio.component/Contents/SharedSupport/SystemSounds/system/Screen Capture.aif` (fallback `Grab.aif`) using `/usr/bin/afplay`.
- **Update**: Mounts `macos-universal.dmg` via `hdiutil attach -nobrowse -readonly`, deletes `/Applications/iris.app`, copies the bundle with `cp -R`, and unmounts via `hdiutil detach`.
