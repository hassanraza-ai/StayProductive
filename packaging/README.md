# Packaging

Distribution files for a system install. Nothing here touches existing user
data, enables a service, or installs a browser extension: the package is
unprivileged user software plus an optional per-user daemon unit.

## Layout

| Path | Purpose |
| --- | --- |
| `Makefile` | `build`, `install`, `uninstall`, `check`; honours `DESTDIR` and `PREFIX` |
| `launcher/` | installed `stayproductive` and `stayproductive-x11` launchers, with `@LIBDIR@` substituted at install time |
| `desktop/` | freedesktop application entry |
| `icons/` | scalable application icon |
| `metainfo/` | AppStream metadata |
| `systemd/` | user unit template; installed but **not enabled** |
| `deb/` | Debian package builder using `dpkg-deb` |
| `rpm/` | RPM spec |
| `arch/` | `PKGBUILD` |
| `tests/` | layout and metadata checks that run without a package toolchain |

Installed layout under `/usr`:

```
/usr/bin/stayproductive
/usr/bin/stayproductive-daemon
/usr/bin/stayproductive-x11
/usr/lib/stayproductive/stayproductive/*.py        UI package
/usr/lib/systemd/user/stayproductive.service       disabled by default
/usr/share/applications/io.github.stayproductive.StayProductive.desktop
/usr/share/icons/hicolor/scalable/apps/io.github.stayproductive.StayProductive.svg
/usr/share/metainfo/io.github.stayproductive.StayProductive.metainfo.xml
/usr/share/stayproductive/gnome-extension/         copied per user by Settings
/usr/share/stayproductive/browser-extension/       extension, host, installer
/usr/share/doc/stayproductive/
```

The UI finds the daemon through `PATH`; the daemon finds the X11 adapter through
`PATH`. No repo checkout is required after installation.

## Debian

```sh
packaging/deb/build.sh
sudo apt install ./packaging/out/stayproductive_*_*.deb
```

The build needs `dpkg-deb`, a Rust toolchain, and Python 3. It assembles the
tree in a temporary directory and runs `dpkg-deb --info` on the result.

Dependencies declared: `python3 (>= 3.10)`, `python3-gi`,
`gir1.2-gtk-4.0`, `gir1.2-adw-1`, `systemd`. `gnome-shell` is recommended, not
required, because the X11 adapter works without it.

## RPM and Arch

```sh
rpmbuild -ba packaging/rpm/stayproductive.spec        # from a release tarball
cd packaging/arch && makepkg -si                     # from a release tarball
```

Both specs call the same `make install`, so the file list stays identical. They
have not been built on this machine; the `%files` list is checked by the layout
test, not by a real `rpmbuild` or `makepkg` run.

## Manual install

```sh
sudo make -C packaging install PREFIX=/usr
```

For a per-user prefix:

```sh
make -C packaging install PREFIX="$HOME/.local"
```

`make uninstall` removes the installed files and leaves `state.db`, profiles,
commitments, and history untouched.

## Service

The unit is installed for reference and for `systemctl --user enable`. The app
still writes its own unit when **Settings → Background service → Start at
login** is used, pointing at the daemon in `PATH`; the two describe the same
service, so do not use both. The packaged unit runs
`/usr/bin/stayproductive-daemon --socket` with `Restart=always` and
`NoNewPrivileges=yes`.

Enable it explicitly:

```sh
systemctl --user daemon-reload
systemctl --user enable --now stayproductive.service
```

## Browser extension after install

The extension files are installed under `/usr/share/stayproductive/`. Load
`extension/` as an unpacked extension, then install the native host from the
same tree:

```sh
python3 /usr/share/stayproductive/browser-extension/install.py \
  --browser chrome --extension-id <id>
```

This is a per-user operation. Firefox uses the fixed `stayproductive@local` ID.

## Flatpak

`flatpak/io.github.stayproductive.StayProductive.json` builds the daemon and UI
against `org.gnome.Platform//49` with the Rust SDK extension:

```sh
flatpak-builder --user --install --force-clean build-dir \
  packaging/flatpak/io.github.stayproductive.StayProductive.json
```

The manifest uses `--share=network` during the build so Cargo can fetch crates.
That is acceptable for a local build but not for Flathub; a submission needs
`cargo-sources.json` vendoring and a `--offline` build. **The manifest has not
been built on this machine** because `flatpak-builder` is unavailable, so treat
it as the documented starting point, not as tested packaging.

Flatpak limits this build in ways the native packages do not:

- the daemon runs as a child of the window unless the sandbox is given a
  systemd user service; closing the window stops enforcement;
- browser native messaging cannot launch a host inside the sandbox from a
  browser outside it, so the companion extension is unavailable;
- the GNOME Shell extension lives on the host and cannot be installed from the
  sandbox automatically.

For full enforcement and the browser layer, use the deb/rpm/Arch packages or
the source checkout. The architecture note (`ADR-004`) already treats Flatpak
as extension-free and DNS-free; this README makes the daemon-lifetime limit
explicit as well.

## Verification

```sh
make -C packaging check
make -C packaging install DESTDIR=/tmp/stage PREFIX=/usr
desktop-file-validate packaging/desktop/io.github.stayproductive.StayProductive.desktop
appstreamcli validate --no-net packaging/metainfo/io.github.stayproductive.StayProductive.metainfo.xml
python3 -m unittest discover -s packaging/tests -v
```

`make check` needs `desktop-file-validate` and `appstreamcli`; the layout test
itself only needs Python.
