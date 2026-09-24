Name:           iris
Version:        0.1.0
Release:        1%{?dist}
Summary:        Screenshot and screen-recording utility

License:        MIT OR Apache-2.0
URL:            https://github.com/santhreal/iris
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  gcc
BuildRequires:  rust >= 1.90
BuildRequires:  cargo
BuildRequires:  pipewire-devel >= 0.3.0
BuildRequires:  libxkbcommon-devel
BuildRequires:  libxkbcommon-x11-devel
BuildRequires:  fontconfig-devel
BuildRequires:  wayland-devel
BuildRequires:  libX11-devel
BuildRequires:  libxcb-devel

# The binary links pipewire, xcb, and xkbcommon; it loads Vulkan (the
# renderer), Wayland, fontconfig, and EGL/GLES (Wayland recording) at
# run time, which no automatic dependency scan sees.
Requires:       pipewire-libs >= 0.3.0
Requires:       libxkbcommon
Requires:       libxkbcommon-x11
Requires:       libxcb
Requires:       fontconfig
Requires:       libwayland-client
Requires:       vulkan-loader
Requires:       libglvnd-egl
Requires:       libglvnd-gles
Recommends:     wl-clipboard
Recommends:     xclip

# Disable automatic debuginfo generation when packaging pre-stripped binary
%global debug_package %{nil}

%description
iris is a native, lightweight screen capture and screen recording
utility built with GPUI. It runs a background daemon with tray
controls, global hotkeys, and an IPC listener, paired with region
capture, window capture, screen recording via PipeWire, and an
annotation editor.

%prep
%autosetup -n %{name}-%{version}

%build
# When building from source:
if [ ! -f iris ] && [ ! -f target/release/iris ]; then
    cargo build --release
fi

%install
rm -rf %{buildroot}

# Install executable binary
install -d %{buildroot}%{_bindir}
if [ -f iris ]; then
    install -m 0755 iris %{buildroot}%{_bindir}/%{name}
elif [ -f target/release/iris ]; then
    install -m 0755 target/release/iris %{buildroot}%{_bindir}/%{name}
else
    echo "Error: iris executable not found" >&2
    exit 1
fi

# Install desktop files
install -d %{buildroot}%{_datadir}/applications
install -m 0644 packaging/linux/dev.iris.app.desktop %{buildroot}%{_datadir}/applications/dev.iris.app.desktop

install -d %{buildroot}%{_sysconfdir}/xdg/autostart
install -m 0644 packaging/linux/iris-autostart.desktop %{buildroot}%{_sysconfdir}/xdg/autostart/%{name}-autostart.desktop

# Install AppStream metainfo
if [ -f packaging/linux/dev.iris.app.metainfo.xml ]; then
    install -d %{buildroot}%{_datadir}/metainfo
    install -m 0644 packaging/linux/dev.iris.app.metainfo.xml %{buildroot}%{_datadir}/metainfo/dev.iris.app.metainfo.xml
fi

# Install icons in hicolor theme
for size in 16 24 32 48 64 128 256 512 1024; do
    if [ -f packaging/icons/iris-${size}.png ]; then
        install -d %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps
        install -m 0644 packaging/icons/iris-${size}.png %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/%{name}.png
    fi
done

# Ensure standard 256 and 512 are installed
if [ -f packaging/icons/iris-256.png ]; then
    install -d %{buildroot}%{_datadir}/icons/hicolor/256x256/apps
    install -m 0644 packaging/icons/iris-256.png %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/%{name}.png
fi
if [ -f packaging/icons/iris-512.png ]; then
    install -d %{buildroot}%{_datadir}/icons/hicolor/512x512/apps
    install -m 0644 packaging/icons/iris-512.png %{buildroot}%{_datadir}/icons/hicolor/512x512/apps/%{name}.png
fi

%post
touch --no-create %{_datadir}/icons/hicolor &>/dev/null || :
update-desktop-database &>/dev/null || :

%postun
if [ $1 -eq 0 ]; then
    touch --no-create %{_datadir}/icons/hicolor &>/dev/null || :
    gtk-update-icon-cache %{_datadir}/icons/hicolor &>/dev/null || :
fi
update-desktop-database &>/dev/null || :

%posttrans
gtk-update-icon-cache %{_datadir}/icons/hicolor &>/dev/null || :

%files
%{_bindir}/%{name}
%{_datadir}/applications/dev.iris.app.desktop
%{_sysconfdir}/xdg/autostart/%{name}-autostart.desktop
%{_datadir}/icons/hicolor/*/apps/%{name}.png
%{_datadir}/metainfo/dev.iris.app.metainfo.xml

%changelog
* Mon Sep 21 2026 Santh <64453045+santhreal@users.noreply.github.com> - 0.1.0-1
- Initial release of iris 0.1.0
