Name:           listello
Version:        0.1.0
Release:        1%{?dist}
Summary:        Widgets and a Dock for the MacBook Pro Touch Bar
License:        MIT AND Apache-2.0
URL:            https://github.com/atsetilam/listello
Source0:        %{name}-%{version}.tar

# Rust 1.92 or newer is needed; Fedora's own cargo is recent enough (see docs/INSTALL.md).
# The build downloads the crates in Cargo.lock, so it needs the network.
BuildRequires:  make
BuildRequires:  gcc-c++
BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  clang
BuildRequires:  clang-devel
# Fedora's default linker (GNU ld) fails on the Qt link step; lld does not. Apple Silicon
# needs it anyway.
BuildRequires:  lld
BuildRequires:  pkgconf-pkg-config
BuildRequires:  qt6-qtbase-devel
BuildRequires:  qt6-qtdeclarative-devel
BuildRequires:  cairo-devel
BuildRequires:  dbus-devel
BuildRequires:  fontconfig-devel
BuildRequires:  freetype-devel
BuildRequires:  libinput-devel
BuildRequires:  pipewire-devel
BuildRequires:  librsvg2-devel
BuildRequires:  systemd-devel
BuildRequires:  systemd-rpm-macros

Requires:       rsms-inter-fonts
Requires:       kf6-kirigami
Requires:       kf6-qqc2-desktop-style
Requires:       qt6-qtdeclarative
# listello-core replaces tiny-dfr and ships the same udev rules and unit-file names under its
# own names; having both installed would start two daemons on one panel.
Conflicts:      tiny-dfr

# Rust binaries; there is nothing useful to split out.
%global debug_package %{nil}

%description
listello draws function keys, media keys, widgets and an app Dock on the Touch Bar of
an Intel (T2) or Apple Silicon MacBook Pro, and has a settings window to choose what it
shows. It is a fork of tiny-dfr and replaces it.

Installing the package turns nothing on. The administrator starts the system side with
"systemctl enable --now listello-core.socket"; each user then starts listello from the
application menu, and chooses there whether it starts at login.

%prep
%autosetup -c

%build
make fetch
make build

%install
make install DESTDIR=%{buildroot} LICENSEDIR=%{_licensedir}/%{name}

# Nothing is enabled, started, stopped or restarted for anybody: the administrator enables
# listello-core.socket, and each user starts the agent from the settings window.
%post
systemctl daemon-reload >/dev/null 2>&1 || :
udevadm control --reload >/dev/null 2>&1 || :
if [ "$1" -eq 1 ]; then
    cat <<'MSG'
listello is installed. Nothing starts by itself. To use it:

  sudo systemctl enable --now listello-core.socket

then open "listello" from the application menu. The first time, it asks whether to start
listello every time you log in.
MSG
else
    cat <<'MSG'
listello was upgraded. Stop the renderer; it starts again by itself the next time listello
is started:

  sudo systemctl stop listello-core
MSG
fi

%preun
%systemd_preun listello-core.socket

%postun
systemctl daemon-reload >/dev/null 2>&1 || :
udevadm control --reload >/dev/null 2>&1 || :

%files
%license %{_licensedir}/%{name}/LICENSE
%license %{_licensedir}/%{name}/LICENSE.material
%license %{_licensedir}/%{name}/LICENSE.lucide
%{_bindir}/listello-core
%{_bindir}/listello-agent
%{_bindir}/listello-settings
%{_datadir}/listello/
%{_datadir}/applications/org.listello.Agent.desktop
%{_datadir}/applications/org.listello.Settings.desktop
%{_datadir}/icons/hicolor/scalable/apps/listello.svg
%{_unitdir}/listello-core.service
%{_unitdir}/listello-core.socket
%{_unitdir}/systemd-backlight@*.service
%{_userunitdir}/listello-agent.service
%{_prefix}/lib/udev/rules.d/99-touchbar-listello.rules
%{_prefix}/lib/udev/rules.d/99-touchbar-seat.rules

%changelog
* Thu Oct 22 2026 Giulio Malitesta <atsetilam@gmail.com> - 0.1.0-1
- First release.
