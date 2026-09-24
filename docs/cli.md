# Command-Line Interface

`iris` operates as a single-instance daemon. Commands forward over local interprocess communication (IPC) to an active daemon, or spawn a detached daemon when none is running. On Windows a terminal does not wait for `iris.exe`; see [Windows](platforms.md#windows).

## Command Reference

| Flag | Argument | Description |
| --- | --- | --- |
| `--capture` | None | Open region capture overlay, then display [toast notification](toast.md#toast-notifications). |
| `--capture-fullscreen` | None | Capture entire virtual display without opening overlay, then display [toast notification](toast.md#toast-notifications). |
| `--capture-window` | None | Capture focused window, then display [toast notification](toast.md#toast-notifications). |
| `--delay` | `<secs>` | Wait specified integer seconds (`u64`), capture entire virtual display, then display [toast notification](toast.md#toast-notifications). |
| `--record-window` | None | Toggle window recording. Prompts for window selection, or stops active recording. |
| `--record-region` | None | Open region selection overlay to record selected screen rectangle. |
| `--record-pause` | None | Pause or resume active screen recording. Requires running daemon. |
| `--record-mic` | None | Toggle microphone capture on active recording. Requires running daemon. Returns an error for a GIF recording, which has no audio track. |
| `--library` | None | Open [capture library](library.md#capture-library) window. |
| `--settings` | None | Open [settings window](configuration.md#settings-window). |
| `--home` | None | Open home window displaying quick action buttons. |
| `--annotate` | `<file>` | Open [annotation editor](editor.md#annotation-editor) for the image at `<file>`. |
| `--toast` | `<file>` | Display [toast notification](toast.md#toast-notifications) for the image at `<file>`. |
| `--quit` | None | Terminate running daemon process after saving any active recording. Requires running daemon. |
| `--version` | None | Print version string to standard output and exit. |
| `--check-update` | None | Query GitHub releases API for newer version and print status. |
| `--update` | None | Download and apply latest release asset for current platform, then restart daemon. |
| `-h`, `--help` | None | Print the option list to standard output and exit. |

A relative `<file>` resolves against the working directory of the `iris` command, not the daemon's. The file must exist.

An unknown option, a positional argument, an option without its value, a `--delay` value that is not a whole number, or a `<file>` that is not an existing file prints one line per error to standard error, then `iris: run 'iris --help' for the options`, and exits with status 2. Nothing is sent to a running daemon and no daemon starts. `--help` prints the option list even when other arguments are invalid.

A running daemon ignores an option it does not recognize in a forwarded command line, such as one sent by a newer `iris` binary, and runs the rest.

## Process Model and Interprocess Communication

The process model differentiates between client commands, daemon processes, and live-daemon-only commands.

### Interprocess Communication Transport

IPC uses a local socket implementation:
- Linux and macOS: Unix domain socket at `$XDG_RUNTIME_DIR/iris.sock`. If `$XDG_RUNTIME_DIR` is unset, the socket path is `<temp_dir>/iris.sock`. Socket file permissions are set to `0600`.
- Windows: named pipe at `\\.\pipe\iris`.

### Invocations Without Running Daemon

1. Bare invocation (`iris` without arguments):
   The process runs in the foreground as the daemon. It initializes the single-instance IPC listener, system tray icon, and global hotkeys. It does not open the home window.
2. Live-daemon-only commands (`--quit`, `--record-pause`, `--record-mic`):
   The client attempts to connect to the socket and does not spawn a daemon. When no daemon answers, `--quit` exits with status 0; `--record-pause` and `--record-mic` print `iris: no iris daemon is running` to standard error and exit with status 1.
3. Other flagged commands (`--capture`, `--library`, `--settings`, etc.):
   The client spawns a detached background daemon process with standard streams redirected to null. The client polls the IPC socket for up to 8 seconds. Once the socket accepts connections, the client transmits the options, each `<file>` as an absolute path, separated by newlines and exits with status 0. If the daemon process fails to spawn, the client runs the daemon and executes the commands in the foreground. If the spawned daemon does not bind the socket within 8 seconds, the client prints `iris: the daemon did not start within 8s; see <log file>` to standard error and exits with status 1.

### Invocations With Running Daemon

1. Bare invocation (`iris` without arguments):
   The client connects to the socket, transmits `--home` to display the home window, and exits with status 0.
2. Flagged invocations:
   The client connects to the socket, transmits the options, each `<file>` as an absolute path, separated by newlines, and exits with status 0. The daemon receives the options and dispatches the corresponding tasks.

### Local Client Commands

`--help`, `--version`, `--check-update`, and `--update` execute locally in the client process without communicating with the daemon socket. When one command line holds several, only the first of `--help`, `--version`, `--check-update`, `--update` in that order runs, and no other option on the line is sent to the daemon:
- `--help` prints the option list and exits with status 0.
- `--version` prints `iris <version>` to standard output and exits with status 0.
- `--check-update` queries `https://api.github.com/repos/santhreal/iris/releases/latest`. If a newer release exists, it prints `iris: update available: <version>` and exits with status 0. If the binary is current, it prints `iris: up to date (<version>)` and exits with status 0. On network or parsing failure, it prints the error to standard error and exits with status 1.
- `--update` checks for a newer release. If current, it prints `iris: up to date (<version>)` and exits with status 0. If a newer release exists, it downloads the platform asset to the `update` directory under the [cache directory](configuration.md), sends `--quit` to any running daemon, waits up to 5 seconds for the socket to release, applies the replacement file, and relaunches the executable. On failure, it prints the error to standard error and exits with status 1.

A local command whose standard output cannot be written, such as a pipe whose reader has exited, exits with status 1 and prints nothing to standard error.

## Window Manager and Compositor Keybindings

On [Wayland](platforms.md#linux-wayland), X11 root-window global key grabs are unavailable. Wayland compositors must bind shortcuts to `iris` command invocations, which forward commands to the daemon over the IPC socket.

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
bind = CTRL SHIFT, R, exec, iris --record-window

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
bindsym Ctrl+Shift+r exec iris --record-window
bindsym Ctrl+Mod1+r exec iris --record-region
bindsym Mod4+l exec iris --library
```

### i3 (`~/.config/i3/config`)

```ini
bindsym Print exec --no-startup-id iris --capture
bindsym Shift+Print exec --no-startup-id iris --capture-fullscreen
bindsym Mod1+Print exec --no-startup-id iris --capture-window
bindsym Ctrl+Shift+r exec --no-startup-id iris --record-window
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
ctrl + shift + r
    iris --record-window

# Record region
ctrl + alt + r
    iris --record-region

# Open library
super + l
    iris --library
```
