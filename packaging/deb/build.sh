#!/bin/sh
# Build a Debian package from the current checkout.
#
#   packaging/deb/build.sh
#
# The result lands in packaging/out/ and is validated with dpkg-deb --info.
# No root is needed; the package is assembled in a temporary DESTDIR.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)
ARCH=${ARCH:-$(dpkg --print-architecture)}
STAGE=$(mktemp -d "${TMPDIR:-/tmp}/stayproductive-deb.XXXXXX")
OUT="$ROOT/packaging/out"
trap 'rm -rf "$STAGE"' EXIT INT TERM

make -C "$ROOT/packaging" install DESTDIR="$STAGE" PREFIX=/usr
chmod 755 "$STAGE"

SIZE=$(du -sk "$STAGE/usr" | cut -f1)
install -d "$STAGE/DEBIAN"
cat > "$STAGE/DEBIAN/control" <<EOF
Package: stayproductive
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Installed-Size: $SIZE
Maintainer: StayProductive contributors <maintainers@stayproductive.invalid>
Homepage: https://github.com/hassanraza-ai/StayProductive
Description: Local-first focus sessions and distraction blocking
 StayProductive plans focus profiles with schedules, application and
 website rules, daily and hourly limits, and strict commitments. A
 background user service keeps enforcing after the window is closed.
 .
 Blocking works on X11 and through an optional GNOME Shell extension.
 Website host rules and keyword replacement work in Chromium browsers
 through the bundled companion extension.
Depends: python3 (>= 3.10), python3-gi, gir1.2-gtk-4.0, gir1.2-adw-1, systemd
Recommends: gnome-shell
EOF

install -d "$OUT"
PACKAGE="$OUT/stayproductive_${VERSION}_${ARCH}.deb"
dpkg-deb --build --root-owner-group "$STAGE" "$PACKAGE"

echo
echo "Built $PACKAGE"
dpkg-deb --info "$PACKAGE"
