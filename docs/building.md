# Building

```sh
cargo build --release --bin iris
```

The executable is written to `target/release/iris` (`target\release\iris.exe` on Windows). Building requires Rust 1.90 or newer.

## Build Dependencies

### Ubuntu and Debian

```sh
sudo apt-get install -y \
  build-essential pkg-config cmake clang \
  libasound2-dev libfontconfig-dev libglib2.0-dev libssl-dev \
  libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev libxcb1-dev \
  libvulkan1 libegl1-mesa-dev libgl1-mesa-dev \
  libpipewire-0.3-dev libspa-0.2-dev xdg-desktop-portal
```

### Fedora and RHEL

```sh
sudo dnf install -y \
  gcc gcc-c++ make cmake clang pkgconf-pkg-config \
  alsa-lib-devel fontconfig-devel glib2-devel openssl-devel \
  wayland-devel libxcb-devel libxkbcommon-x11-devel \
  vulkan-loader mesa-libEGL-devel mesa-libGL-devel \
  pipewire-devel
```

### Windows

Install the `x86_64-pc-windows-msvc` Rust toolchain and the Visual Studio Build Tools with the Windows SDK. `build.rs` compiles `packaging/windows/iris.rc`, the icon and version resource, with the SDK's resource compiler.

### macOS

Install the Xcode Command Line Tools:

```sh
xcode-select --install
```

## Packages

Each packaging script takes the executable with `--bin` (`--binary` on Windows) and writes the package and its `.sha256` sidecar to the directory given with `--out`. The version is read from `Cargo.toml`. `packaging/CONTRACT.md` lists the asset names and the install layout.

### Linux

```sh
bash packaging/linux/build_deb.sh --bin target/release/iris --out dist
bash packaging/linux/build_rpm.sh --bin target/release/iris --out dist
bash packaging/linux/build_appimage.sh --bin target/release/iris --out dist
```

`build_deb.sh` requires `dpkg-deb` and `readelf`, and sets the package's glibc dependency from the newest glibc symbol version the executable uses. `build_rpm.sh` requires `rpmbuild` (the `rpm` package on Debian and Ubuntu, `rpm-build` on Fedora). `build_appimage.sh` runs `appimagetool` from `PATH`, from `--appimagetool <path>`, or downloads it to `.build-staging/`.

The release workflow (`.github/workflows/package.yml`) builds the Linux executable in the `rust:1-bookworm` container (Debian 12). That executable uses no glibc symbol version newer than 2.35, so the packages install on glibc 2.35 and later. An executable built on a newer system requires that system's glibc.

`packaging/linux/check_installed.sh` starts an installed iris on a private Xvfb display, waits for the home window, and quits it. With `--upgrade <command>`, it runs the command while the daemon runs, checks that the daemon still runs on the replaced binary, quits it with the upgraded iris, and starts the upgraded iris. To check an upgrade of an installed deb to a package of the same binary at a higher version:

```sh
bash packaging/linux/build_deb.sh --bin target/release/iris --version 0.1.0.1 --out dist/next
bash packaging/linux/check_installed.sh --upgrade "sudo apt-get install -y ./dist/next/iris-0.1.0.1-linux-x86_64.deb"
```

### Windows

In Git Bash, with NSIS's `makensis` on `PATH`:

```sh
cargo build --locked --release --bin iris
bash packaging/windows/build_installer.sh --binary target/release/iris.exe --out dist
bash packaging/windows/build_portable.sh --binary target/release/iris.exe --out dist
```

### macOS

The release DMG holds a universal executable:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --locked --release --target aarch64-apple-darwin --target x86_64-apple-darwin
mkdir -p target/universal-apple-darwin/release
lipo -create -output target/universal-apple-darwin/release/iris \
  target/aarch64-apple-darwin/release/iris \
  target/x86_64-apple-darwin/release/iris
packaging/macos/make_app.sh --bin target/universal-apple-darwin/release/iris --out dist
packaging/macos/make_dmg.sh --app dist/iris.app --out dist
```

## Tests

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

The recorder tests encode and probe files with `ffmpeg` and `ffprobe` from `PATH`.

The window tests on Linux open windows on the X server named by `IRIS_X11_TEST_DISPLAY` and are skipped when it is unset. Run them on a private Xvfb server:

```sh
xvfb-run -a -s '-screen 0 1600x1000x24' \
  sh -c 'IRIS_X11_TEST_DISPLAY=$DISPLAY cargo test --locked'
```

On a machine without a GPU, set `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json` to render with lavapipe (`mesa-vulkan-drivers`).

The tray test starts a private session bus with `dbus-daemon` from `PATH`.
