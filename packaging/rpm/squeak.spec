Name:           squeak
Version:        0.1.0
Release:        1%{?dist}
Summary:        Open-source HID++ 2.0 peripheral configurator, daemon, and CLI

License:        GPL-3.0-or-later
URL:            https://github.com/0x70776c/Squeak
Source0:        %{url}/archive/refs/tags/v%{version}.tar.gz

BuildRequires:  cargo >= 1.89
BuildRequires:  rust >= 1.89
BuildRequires:  pkgconfig(gtk4) >= 4.6
BuildRequires:  pkgconfig(libadwaita-1) >= 1.1
BuildRequires:  systemd-rpm-macros
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib

Requires:       gtk4
Requires:       libadwaita
Requires:       polkit

%description
Squeak is an open-source Linux desktop application, D-Bus daemon (squeakd),
and CLI (squeak) for configuring HID++ 2.0 pointing devices without proprietary
software.

%prep
%autosetup -n Squeak-%{version}

%build
cargo build --release --workspace --locked

%install
install -Dm755 target/release/squeakd %{buildroot}%{_bindir}/squeakd
install -Dm755 target/release/squeak %{buildroot}%{_bindir}/squeak
install -Dm755 target/release/squeak-gui %{buildroot}%{_bindir}/squeak-gui

install -Dm644 data/applications/dev.squeak.Squeak.desktop %{buildroot}%{_datadir}/applications/dev.squeak.Squeak.desktop
install -Dm644 data/metainfo/dev.squeak.Squeak.metainfo.xml %{buildroot}%{_metainfodir}/dev.squeak.Squeak.metainfo.xml
install -Dm644 data/icons/hicolor/scalable/apps/dev.squeak.Squeak.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/dev.squeak.Squeak.svg
install -Dm644 data/icons/hicolor/128x128/apps/dev.squeak.Squeak.png %{buildroot}%{_datadir}/icons/hicolor/128x128/apps/dev.squeak.Squeak.png
install -Dm644 data/icons/hicolor/256x256/apps/dev.squeak.Squeak.png %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/dev.squeak.Squeak.png
install -Dm644 data/icons/hicolor/512x512/apps/dev.squeak.Squeak.png %{buildroot}%{_datadir}/icons/hicolor/512x512/apps/dev.squeak.Squeak.png

install -Dm644 data/dbus/dev.squeak.Daemon.service %{buildroot}%{_datadir}/dbus-1/services/dev.squeak.Daemon.service
install -Dm644 data/systemd/squeakd.service %{buildroot}%{_userunitdir}/squeakd.service

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/dev.squeak.Squeak.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/dev.squeak.Squeak.metainfo.xml

%post
%systemd_user_post squeakd.service

%preun
%systemd_user_preun squeakd.service

%files
%license LICENSE
%doc README.md CHANGELOG.md
%{_bindir}/squeakd
%{_bindir}/squeak
%{_bindir}/squeak-gui
%{_datadir}/applications/dev.squeak.Squeak.desktop
%{_metainfodir}/dev.squeak.Squeak.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/dev.squeak.Squeak.svg
%{_datadir}/icons/hicolor/128x128/apps/dev.squeak.Squeak.png
%{_datadir}/icons/hicolor/256x256/apps/dev.squeak.Squeak.png
%{_datadir}/icons/hicolor/512x512/apps/dev.squeak.Squeak.png
%{_datadir}/dbus-1/services/dev.squeak.Daemon.service
%{_userunitdir}/squeakd.service

%changelog
* Fri Nov 13 2026 Squeak Contributors <maintainers@squeak.dev> - 0.1.0-1
- Initial 0.1.0 release
