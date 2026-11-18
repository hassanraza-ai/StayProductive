Name:           stayproductive
Version:        0.1.0
Release:        1%{?dist}
Summary:        Local-first focus sessions and distraction blocking
License:        Apache-2.0
URL:            https://github.com/hassanraza-ai/StayProductive
Source0:        %{url}/archive/v%{version}/StayProductive-%{version}.tar.gz
ExclusiveArch:  x86_64 aarch64

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  desktop-file-utils
BuildRequires:  python3-devel
Requires:       python3 >= 3.10
Requires:       python3-gobject
Requires:       gtk4
Requires:       libadwaita
Requires:       systemd

%description
StayProductive plans focus profiles with schedules, application and website
rules, daily and hourly limits, and strict commitments. A background user
service keeps enforcing after the window is closed.

Application blocking works on X11 and through an optional GNOME Shell
extension. Website host rules and keyword replacement work in Chromium
browsers through the bundled companion extension.

%prep
%autosetup -n StayProductive-%{version}

%build
cargo build --release --locked -p stayproductive-daemon

%install
make -C packaging install DESTDIR=%{buildroot} PREFIX=%{_prefix}

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/io.github.stayproductive.StayProductive.desktop

%files
%{_bindir}/stayproductive
%{_bindir}/stayproductive-daemon
%{_bindir}/stayproductive-x11
%{_libdir}/stayproductive
%{_datadir}/applications/io.github.stayproductive.StayProductive.desktop
%{_datadir}/icons/hicolor/scalable/apps/io.github.stayproductive.StayProductive.svg
%{_datadir}/metainfo/io.github.stayproductive.StayProductive.metainfo.xml
%{_datadir}/stayproductive
%{_userunitdir}/stayproductive.service
%doc %{_datadir}/doc/stayproductive

%changelog
* Wed Nov 18 2026 StayProductive contributors <maintainers@stayproductive.invalid> - 0.1.0-1
- First packaged build.
