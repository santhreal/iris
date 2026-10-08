# Updates

iris updates itself from the releases of [santhreal/iris](https://github.com/santhreal/iris/releases) on GitHub. The daemon checks for a newer release in the background and offers it in the tray menu, in a notice, and in the Updates pane of the settings window. `iris --update` installs it from a terminal.

```sh
iris --check-update     # print whether a newer release exists
iris --update           # download, verify, and install it
```

`--check-update` prints `iris: update available: <version>` or `iris: up to date (<version>)`. `--update` prints `iris: up to date (<version>)` when no newer release exists, and otherwise installs it as [Install](#install) describes.

## Channels

The `update_channel` key of `config.toml` ([Configuration](configuration.md#updates)) selects the releases a check offers:

| Channel | Releases offered |
| --- | --- |
| `stable` (default) | GitHub's latest release. A draft or a prerelease is never offered. |
| `beta` | The release with the highest version among the 30 newest releases, prerelease or not. |

A release is a candidate on `beta` when it is not a draft, its tag is `v` followed by a semantic version (`v0.2.0`, `v0.2.0-beta.1`), and it holds an asset for this install. A `beta` install therefore receives stable releases too: `0.2.0` is newer than `0.2.0-rc.1`. Versions compare by semantic version precedence, not by release date.

A check offers a release only when its version is newer than the running iris. A newer release that lacks the asset's `.sha256` sidecar or `.minisig` signature is reported as an error and offers nothing.

The asset for this install is the one whose name ends in the install's kind and architecture:

| Install | Asset |
| --- | --- |
| Windows, installed with the setup | `iris-<version>-windows-<arch>-setup.exe` |
| Windows, portable | `iris-<version>-windows-<arch>-portable.zip` |
| macOS | `iris-<version>-macos-universal.dmg` |
| Linux AppImage | `iris-<version>-linux-<arch>.AppImage` |
| Linux deb | `iris-<version>-linux-<arch>.deb` |
| Linux rpm | `iris-<version>-linux-<arch>.rpm` |

`<arch>` is `x86_64` or `aarch64`, the architecture iris was built for.

## Background Check

While `check_for_updates` is `true` (the default), the daemon checks about 60 seconds after it starts and then every 24 hours. Each check that gets an answer from GitHub is recorded in the [state file](#state-file), whether the daemon, **Check Now**, `iris --check-update`, or `iris --update` ran it. A daemon that starts within 24 hours of the last recorded check on the same channel waits until those 24 hours have passed.

The daemon reads `check_for_updates` and `update_channel` at least every 5 minutes, so a change in the settings window takes effect within that time. A change of `update_channel` makes a check due at once.

A check that fails, such as one made without a network connection, is tried again every hour. The first failure of a run writes one line to `iris.log`:

```text
iris: update check failed, retrying every 60 min: <error>
```

The next check that gets an answer writes `iris: update check: GitHub answered again`. A failed background check shows no notice and leaves the tray menu as it was.

Set `check_for_updates = false` to stop the background check:

```toml
check_for_updates = false
```

With it off, the daemon checks nothing and hides the tray menu's install row. **Check Now**, `iris --check-update`, and `iris --update` still check when you run them.

## Offers

When a check finds a newer release:

- The tray menu shows **Install iris** *version* between **Settings** and **Quit** ([Usage](usage.md)). The row is hidden while no newer release is known.
- The daemon shows a notice titled `iris <version> is available` with an **Install** button. Each version shows this notice once, across daemon restarts.
- The Updates pane of the settings window shows **Install** *version* ([Settings](#settings)).

The offer is the update the last recorded check found, when its version is newer than the running iris and the channel offers it: after a switch from `beta` to `stable`, a prerelease found on `beta` is no longer offered.

## Install

**Install** in the tray menu or the notice downloads and verifies the asset on a thread of the daemon, then starts `iris --update` as a detached process, which installs the verified download without fetching it again. An **Install** while a download runs does nothing. A failure shows in a notice titled `Update failed`.

`iris --update` installs in this order:

1. It checks that this install can replace itself. An install that cannot fails here, before anything is downloaded ([Platform Update Mechanisms](#platform-update-mechanisms)).
2. It downloads the asset and verifies it ([Verification](#verification)).
3. It sends `--quit` to the running daemon and waits up to 60 seconds for it to exit. A quitting daemon saves its recording before it exits.
4. It replaces the installed iris with the download and starts the new iris.

When the daemon still runs 60 seconds after `--quit`, `iris --update` installs nothing, prints `update: the running iris still answers 60 s after --quit; nothing was installed, run iris --update again once it exits`, and exits with status 1.

When the replacement fails after `iris --update` stopped the daemon, it starts the installed iris again as the daemon and appends `; the iris already installed runs again` to the error. When that start fails, it appends `; the iris already installed did not start again, see <log file>`. A daemon that was not running before the update is not started.

`iris --update` writes each error to standard error and to `iris.log` ([paths](configuration.md#iris_home-directory-override)), and exits with status 1.

## Verification

Every release asset is published with two files beside it:

- `<asset>.sha256`: one `sha256sum` line holding the asset's SHA-256 and its file name.
- `<asset>.minisig`: a [minisign](https://jedisct1.github.io/minisign/) signature of the asset by the iris release key, whose trusted comment is `file:<asset> version:<version>`.

An update fetches both before it downloads the asset. The download is installed only when:

- its SHA-256 equals the one the sidecar lists for the asset's file name, and
- its signature verifies against the release key built into iris, the key in [`packaging/minisign.pub`](https://github.com/santhreal/iris/blob/main/packaging/minisign.pub) (key ID `EC8B4C1097043E88`), and
- the signature's trusted comment is `file:<asset> version:<version>` for the asset and the version the check selected.

Each of these fails the update:

| Failure | Error |
| --- | --- |
| No sidecar | the HTTP error for `<asset>.sha256` |
| A sidecar that is not one `sha256sum` line for the asset | ``update: <asset>.sha256 is not one `sha256sum` line for <asset>`` |
| A download whose SHA-256 differs from the sidecar's | `update: the downloaded <asset> has SHA-256 <digest>, its .sha256 sidecar lists <digest>; nothing was installed` |
| A download cut short of the length the server declared | `update: download <asset>: <error>` |
| No signature | the HTTP error for `<asset>.minisig` |
| A file that is not a minisign signature | `update: <asset>.minisig is not a minisign signature: <reason>` |
| A signature by another key | `update: <asset>.minisig is not a release signature: The signature was created with a different key than the one provided` |
| A legacy signature, made without the BLAKE2b-512 prehash `minisign -S` uses | `update: <asset>.minisig is not a release signature: StreamVerifier only supports non-legacy mode signatures` |
| A trusted comment for another asset or another version | `update: <asset>.minisig signs "<comment>", not "file:<asset> version:<version>"` |
| A signature of other bytes, or a trusted comment changed after signing | `update: <asset> does not match its signature <asset>.minisig: The signature verification failed` |

A failed check deletes the download, installs nothing, and leaves the running daemon running. A sidecar or signature that cannot be fetched fails the update before anything is downloaded.

Downloads are kept in the `update` directory under the iris cache directory ([paths](configuration.md#iris_home-directory-override)). The directory is private to the user. When a file of the asset's name is already there and its SHA-256 matches the sidecar, the update verifies its signature and installs it without downloading it again. A file there that fails either check is deleted, and one whose SHA-256 differs is replaced by the download.

## Platform Update Mechanisms

- Windows (installed): An `iris.exe` with `uninstall.exe` beside it downloads `windows-<arch>-setup.exe`, starts it with `/S /RUN` as a detached process that receives no handle of the `iris --update` process, and exits. The installer waits until no process runs the installed `iris.exe`, replaces it, and starts iris. When the installation fails, it starts the `iris.exe` already in place.
- Windows (portable): Any other `iris.exe` downloads `windows-<arch>-portable.zip` and unpacks it with the `tar.exe` Windows ships into `.iris-update` beside `iris.exe`. It renames the running `iris.exe` to `iris.exe.old`, moves the new `iris.exe` into its place, deletes `.iris-update`, and starts the new `iris.exe`. The daemon deletes `iris.exe.old` when it starts. A failed unpack or move deletes `.iris-update` and leaves `iris.exe` as it was. When iris cannot create `.iris-update`, `iris --update` prints `update: cannot write <folder>\.iris-update: <error>; move the portable iris to a folder this account can write, or install it with the setup` and exits with status 1 before downloading anything.
- macOS: Downloads `macos-universal.dmg` and attaches it at a mount point of its own with `hdiutil attach -nobrowse -readonly`. An attach that fails with `Resource temporarily unavailable`, which `hdiutil` reports while another disk image operation runs, is tried up to 30 times, a second apart. It copies the disk image's `iris.app` with `ditto` to `.iris.app.update` beside the bundle iris runs from, detaches the disk image, and swaps the two bundles in one `renamex_np(RENAME_SWAP)` rename, then deletes the old bundle and relaunches the new one. A failure before the swap leaves the installed bundle as it was. An iris outside an `.app` bundle prints `update: <path> is not inside an iris.app bundle; install the release DMG by hand` and exits with status 1 before downloading anything.
- Linux (AppImage): iris runs from an AppImage when the running binary is inside `$APPDIR`, the mount the AppImage runtime sets it to; `$APPIMAGE` then names the AppImage. A program started from another AppImage, such as a terminal, passes that AppImage's `$APPIMAGE` and `$APPDIR` on to the iris it starts, and that iris does not update the other AppImage. The update copies the download beside the AppImage, to its name with the extension `.new` (`iris.AppImage` stages `iris.new`), renames the copy over the AppImage, and launches the new file. A failed copy or rename deletes the copy and leaves the AppImage as it was. When the AppImage's directory takes no new file, `iris --update` prints `update: cannot write <dir>/<stem>.new: <error>; move the AppImage to a folder this account can write, or install the deb or rpm` and exits with status 1 before downloading anything.
- Linux (deb/rpm): An `/usr/bin/iris` that dpkg lists in the `iris` deb downloads `linux-<arch>.deb` and runs `apt-get install -y <file>`. One the `iris` rpm owns downloads `linux-<arch>.rpm` and runs `dnf install -y <file>`. Run as root, `iris --update` runs the command itself. Run as another user, it runs the command through `pkexec`, which prompts for an administrator's password through the session's polkit agent, or on the terminal when no agent runs. The deb and the rpm recommend `pkexec`. When `apt-get` or `dnf` is missing, or `pkexec` is missing for a user other than root, `iris --update` prints the missing program and exits with status 1 before downloading anything. An install that fails prints the command, the reason, and the command that installs the download by hand, for example `update: /usr/bin/pkexec /usr/bin/apt-get install -y <file> was not authorized: the password dialog was dismissed; install it with: sudo apt install <file>`, and the stopped daemon starts again. A successful install starts `/usr/bin/iris` as the daemon. A daemon running while `apt` or `dnf` upgrades the package by hand runs the previous version until it restarts. A **Start iris at login** entry its settings window writes runs the upgraded `/usr/bin/iris`.
- Linux (other): An `iris` that is neither an AppImage nor the `/usr/bin/iris` of the deb or the rpm, such as one built from source, prints `update: <path> is not from an AppImage, deb or rpm; install the release by hand` and exits with status 1 before downloading anything. The running daemon keeps running.

## Settings

The Updates pane of the [settings window](configuration.md#settings-window) (`iris --settings`) holds the **Check for updates automatically** switch (`check_for_updates`), the **Update channel** picker (`update_channel`, **Stable** or **Beta**), the running version, and a status line: `Not checked yet`, `Last checked today at <HH:MM>`, or `Last checked on <YYYY-MM-DD> at <HH:MM>`. **Check Now** queries releases off the UI thread, shows `Checking…`, then `iris is up to date`, `Version <version> is available`, or the error. When a check finds a newer release, **Install** *version* replaces **Check Now**; the pane also shows it on open when the last check found one. **Install** hides the button, downloads the asset and verifies it off the UI thread, showing `Downloading <version>…`, then starts `iris --update` as a detached process and shows `Installing <version>; iris restarts when it is done`. When the download fails, the error shows in the status line and **Install** returns.

## State File

The update check keeps its state in `update.json` beside `iris.log` ([paths](configuration.md#iris_home-directory-override)):

| Platform | Path |
| --- | --- |
| Linux | `$XDG_STATE_HOME/iris/update.json` (`~/.local/state/iris/update.json`) |
| Windows | `%APPDATA%\iris\update.json` |
| macOS | `~/Library/Application Support/iris/update.json` |
| `IRIS_HOME` set | `$IRIS_HOME/state/update.json` |

The file holds the time and channel of the last check that got an answer, the release it found, and the newest version a notice announced. A missing, unreadable, or malformed file, or one written in another format, reads as no state: the daemon checks about 60 seconds after it starts and replaces the file. Deleting the file makes the next daemon check 60 seconds after it starts and show the notice for the release it finds again.

## Verifying a Download by Hand

Download the asset, its `.minisig` signature, and [`packaging/minisign.pub`](https://github.com/santhreal/iris/blob/main/packaging/minisign.pub) into one directory, then run:

```sh
minisign -Vm <asset> -p minisign.pub
```

`minisign` prints `Signature and comment signature verified` and the trusted comment, which must be `file:<asset> version:<version>` for the file you downloaded. [Verifying a Download](install.md#verifying-a-download) lists the download commands, the SHA-256 check, and the steps on macOS and Windows.
