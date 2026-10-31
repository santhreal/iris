# Command-Line Interface

iris is a single-instance app. A command line forwards over local interprocess communication (IPC) to the running iris, or starts iris in the background when none runs. On Windows a terminal does not wait for `iris.exe`; see [Windows](platforms.md#windows).

## Command Reference

| Flag | Argument | Description |
| --- | --- | --- |
| `--capture` | None | Open region capture overlay, then display [toast notification](toast.md#toast-notifications). |
| `--capture-fullscreen` | None | Capture entire virtual display without opening overlay, then display [toast notification](toast.md#toast-notifications). |
| `--capture-window` | None | Capture focused window, then display [toast notification](toast.md#toast-notifications). |
| `--delay` | `<secs>` | Count down the specified integer seconds (`u64`) on a corner card with a cancel button, capture entire virtual display, then display [toast notification](toast.md#toast-notifications) ([Delayed Capture](capture.md#delayed-capture)). |
| `--record-window` | None | Toggle window recording. Prompts for window selection, or stops active recording. |
| `--record-region` | None | Open region selection overlay to record selected screen rectangle. |
| `--record-pause` | None | Pause or resume active screen recording. Requires a running iris. |
| `--record-mic` | None | Toggle microphone capture on active recording. Requires a running iris. Returns an error for a GIF recording, which has no audio track. |
| `--library` | None | Open [capture library](library.md#capture-library) window, or raise and focus the one already open. |
| `--settings` | None | Open [settings window](configuration.md#settings-window), or raise and focus the one already open. |
| `--annotate` | `<file>` | Open [annotation editor](editor.md#annotation-editor) for the image at `<file>`, or raise and focus the editor already open on that file. |
| `--toast` | `<file>` | Display [toast notification](toast.md#toast-notifications) for the image at `<file>`. |
| `--quit` | None | Quit the running iris after saving the active recording and any stopped recording still being joined. Requires a running iris. |
| `--background` | None | Start iris in this process with no window open. When another process runs iris or is starting it, exit with status 0. Takes no other option. |
| `--version` | None | Print version string to standard output and exit. |
| `--check-update` | None | Query GitHub releases API for newer version and print status. |
| `--update` | None | Download and apply latest release asset for current platform, then restart iris. |
| `-h`, `--help` | None | Print the option list to standard output and exit. |

A relative `<file>` resolves against the working directory of the `iris` command, not iris's. The file must exist.

An unknown option, a positional argument, an option without its value, a `--delay` value that is not a whole number, a `<file>` that is not an existing file, or `--background` with another option prints one line per error to standard error, then `iris: run 'iris --help' for the options`, and exits with status 2. Nothing is sent to a running iris and iris does not start. `--help` prints the option list even when other arguments are invalid.

A running iris ignores an option it does not recognize in a forwarded command line, such as one sent by a newer `iris` binary, and runs the rest.

## Process Model and Interprocess Communication

One process runs iris. Every other `iris` invocation is a client: it forwards its command line to that process and exits, or runs a local command.

### Single Instance

One process per user runs iris: the process that holds the instance claim. On Linux and macOS the claim is an exclusive `flock` on `iris.lock` in the directory of `iris.sock`. On Windows it is the named pipe `\\.\pipe\iris-<SID>.lock`. A process takes the claim before it connects to the display, and the operating system releases it when the process exits, however it exits. Only the holder of the claim binds the IPC socket. An iris that was killed leaves `iris.sock` in place, and the next iris replaces it.

Of several `iris` processes started while iris is not running, one takes the claim and becomes iris. Each other process sends its options once that iris binds the socket, as with a running iris, and an `iris --background` among them exits with status 0.

### Interprocess Communication Transport

IPC uses a local socket implementation:
- Linux and macOS: Unix domain socket `iris.sock` in `$XDG_RUNTIME_DIR`, or in the temporary directory when `$XDG_RUNTIME_DIR` is unset or empty. When another user owns that directory or its mode admits other users, as with `/tmp`, the socket is in its subdirectory `iris-<uid>`, which iris creates with mode `0700`. The directory controls access to the socket. When `iris-<uid>` is a symbolic link, is owned by another user, or has a mode that admits other users, the command prints `iris: <dir> is not a directory of uid <uid> with mode 0700; remove it` to standard error and exits with status 1.
- Windows: named pipe `\\.\pipe\iris-<SID>`, where `<SID>` is the security identifier of the account, such as `S-1-5-21-…`. The pipe's owner is that account, and its access control list grants access to that account alone. A client writes nothing to a pipe another account owns. `--quit`, `--record-pause`, and `--record-mic` then act as while iris is not running; any other invocation prints `iris: connect to the running iris: the pipe belongs to <owner SID>, not to this account, <SID>` to standard error and exits with status 1.

iris reads each connection on a thread of its own, up to 16 at once, and closes a connection past that without reading it. A command line longer than 1 MiB, or still arriving 5 seconds after iris accepts the connection, is dropped, and iris runs none of it.

On Linux iris exits when its X server or Wayland compositor exits, as at the end of the desktop session. It saves recordings first; see [Screen Recording](recording.md#window-recording).

### Invocations While iris Is Not Running

1. Bare invocation (`iris` without arguments):
   The process takes the instance claim and runs iris. It connects to the display, loads its fonts, registers the global hotkeys, binds the single-instance IPC socket, and starts the system tray icon, in that order, before it opens any window. A hotkey pressed once the socket accepts connections reaches iris. A command, hotkey, or tray menu item that arrives during startup runs once startup completes. It then opens the library window. When another process holds the claim, the process waits up to 8 seconds for that iris to bind the socket, transmits `--library`, and exits with status 0.
2. Commands for a running iris only (`--quit`, `--record-pause`, `--record-mic`):
   The client attempts to connect to the socket and does not start iris. When no iris answers, `--quit` exits with status 0; `--record-pause` and `--record-mic` print `iris: iris is not running` to standard error and exit with status 1.
3. Other flagged commands (`--capture`, `--library`, `--settings`, etc.):
   The client starts a detached `iris --background` process. On Linux and macOS iris's standard streams are `/dev/null` and it runs in a process group of its own. On Windows iris has no console and inherits no handle from the client. A caller that reads the client's output to its end returns when the client exits. The client polls the IPC socket for up to 8 seconds. Once the socket accepts connections, the client transmits the options, each `<file>` as an absolute path, separated by newlines and exits with status 0. If that process fails to start, the client takes the instance claim, then runs iris and executes the commands itself; when another process holds the claim, the client transmits the options once that iris binds the socket. If iris does not bind the socket within 8 seconds, the client prints `iris: iris did not start within 8s; see <log file>` to standard error and exits with status 1.
4. `iris --background`:
   The process takes the instance claim and runs iris, as a bare invocation does, and opens no window. On Windows it does not attach to the console of the terminal that started it. When another process holds the claim, it exits with status 0. The login autostart entries run `iris --background` ([Installation](install.md)).

### Invocations While iris Runs

1. Bare invocation (`iris` without arguments):
   The client connects to the socket, transmits `--library` to display the library window, and exits with status 0.
2. Flagged invocations:
   The client connects to the socket, transmits the options, each `<file>` as an absolute path, separated by newlines, and exits with status 0. iris receives the options and dispatches the corresponding tasks.
3. `iris --background`:
   The process finds the instance claim held, transmits nothing, and exits with status 0.

### Local Client Commands

`--help`, `--version`, `--check-update`, and `--update` execute locally in the client process without communicating with iris socket. When one command line holds several, only the first of `--help`, `--version`, `--check-update`, `--update` in that order runs, and no other option on the line is sent to iris:
- `--help` prints the option list and exits with status 0.
- `--version` prints `iris <version>` to standard output and exits with status 0.
- `--check-update` queries `https://api.github.com/repos/santhreal/iris/releases/latest`. If a newer release exists, it prints `iris: update available: <version>` and exits with status 0. If the binary is current, it prints `iris: up to date (<version>)` and exits with status 0. On network or parsing failure, or when the newer release has no `.sha256` sidecar for the platform asset, it prints the error to standard error and exits with status 1.
- `--update` checks for a newer release. If current, it prints `iris: up to date (<version>)` and exits with status 0. If a newer release exists, it downloads the platform asset to the `update` directory under the [cache directory](configuration.md) and checks its SHA-256 against the asset's `.sha256` sidecar, sends `--quit` to a running iris, waits up to 60 seconds for it to exit, applies the replacement file, and relaunches the executable. An installed Windows iris starts the installer and exits with status 0, and the installer replaces the file and starts iris; a portable one replaces its own `iris.exe` ([Installation](install.md#updates)). A download that does not match its sidecar is deleted and fails the update before iris is stopped. An iris still running 60 seconds after `--quit` fails the update with nothing installed. On failure, it prints the error to standard error and exits with status 1.

A local command whose standard output cannot be written, such as a pipe whose reader has exited, exits with status 1 and prints nothing to standard error.

## Window Manager and Compositor Keybindings

On [Wayland](platforms.md#linux-wayland), X11 root-window global key grabs are unavailable. iris binds `capture_hotkey` and `record_hotkey` through the GlobalShortcuts portal where the desktop provides one. On a compositor without it, bind shortcuts to `iris` command invocations, which forward commands to iris over the IPC socket.

### Hyprland (`~/.config/hypr/hyprland.conf`)

```ini
# Region capture
bind = , Print, exec, iris --capture

# Fullscreen capture
bind = SHIFT, Print, exec, iris --capture-fullscreen

# Active window capture
bind = CTRL, Print, exec, iris --capture-window

# Fullscreen capture with 5-second delay
bind = ALT, Print, exec, iris --delay 5

# Toggle window recording
bind = CTRL SHIFT, Print, exec, iris --record-window

# Record selected region
bind = CTRL ALT, R, exec, iris --record-region

# Pause/resume active recording
bind = CTRL SHIFT, P, exec, iris --record-pause

# Open library window
bind = $mainMod, L, exec, iris --library
```

### Sway (`~/.config/sway/config`)

```ini
bindsym Print exec iris --capture
bindsym Shift+Print exec iris --capture-fullscreen
bindsym Mod1+Print exec iris --capture-window
bindsym Ctrl+Shift+Print exec iris --record-window
bindsym Ctrl+Mod1+r exec iris --record-region
bindsym Mod4+l exec iris --library
```

### i3 (`~/.config/i3/config`)

```ini
bindsym Print exec --no-startup-id iris --capture
bindsym Shift+Print exec --no-startup-id iris --capture-fullscreen
bindsym Mod1+Print exec --no-startup-id iris --capture-window
bindsym Ctrl+Shift+Print exec --no-startup-id iris --record-window
bindsym Ctrl+Mod1+r exec --no-startup-id iris --record-region
bindsym Mod4+l exec --no-startup-id iris --library
```

### sxhkd (`~/.config/sxhkd/sxhkdrc`)

```sh
# Region screenshot
Print
    iris --capture

# Fullscreen screenshot
shift + Print
    iris --capture-fullscreen

# Active window screenshot
alt + Print
    iris --capture-window

# Record window toggle
ctrl + shift + Print
    iris --record-window

# Record region
ctrl + alt + r
    iris --record-region

# Open library
super + l
    iris --library
```
