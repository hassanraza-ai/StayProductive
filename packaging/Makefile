# Install layout for a distribution package.
#
#   make install DESTDIR=/tmp/stage PREFIX=/usr
#
# The result is relocatable only through PREFIX: the launchers and the systemd
# user unit bake in $(PREFIX) at install time, exactly like the source checkout
# bakes in its own paths.

VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' ../Cargo.toml | head -n1)
PREFIX ?= /usr
DESTDIR ?=
BINDIR = $(PREFIX)/bin
LIBDIR = $(PREFIX)/lib/stayproductive
DATADIR = $(PREFIX)/share
APPDIR = $(DATADIR)/applications
ICONDIR = $(DATADIR)/icons/hicolor/scalable/apps
METAINFODIR = $(DATADIR)/metainfo
EXTDIR = $(DATADIR)/stayproductive
UNITDIR = $(PREFIX)/lib/systemd/user
DOCDIR = $(DATADIR)/doc/stayproductive

.PHONY: all build install uninstall check clean

all: build

build:
	cargo build --release -p stayproductive-daemon

install: build
	install -d "$(DESTDIR)$(BINDIR)"
	install -Dm755 ../target/release/stayproductive-daemon "$(DESTDIR)$(BINDIR)/stayproductive-daemon"
	install -d "$(DESTDIR)$(LIBDIR)/stayproductive"
	cp -R ../crates/ui/stayproductive/. "$(DESTDIR)$(LIBDIR)/stayproductive/"
	find "$(DESTDIR)$(LIBDIR)" -name '__pycache__' -type d -prune -exec rm -rf {} +
	sed 's|@LIBDIR@|$(LIBDIR)|g' launcher/stayproductive.in > "$(DESTDIR)$(BINDIR)/stayproductive"
	sed 's|@LIBDIR@|$(LIBDIR)|g' launcher/stayproductive-x11.in > "$(DESTDIR)$(BINDIR)/stayproductive-x11"
	chmod 755 "$(DESTDIR)$(BINDIR)/stayproductive" "$(DESTDIR)$(BINDIR)/stayproductive-x11"
	install -Dm644 desktop/io.github.stayproductive.StayProductive.desktop \
		"$(DESTDIR)$(APPDIR)/io.github.stayproductive.StayProductive.desktop"
	install -Dm644 icons/hicolor/scalable/apps/io.github.stayproductive.StayProductive.svg \
		"$(DESTDIR)$(ICONDIR)/io.github.stayproductive.StayProductive.svg"
	install -Dm644 metainfo/io.github.stayproductive.StayProductive.metainfo.xml \
		"$(DESTDIR)$(METAINFODIR)/io.github.stayproductive.StayProductive.metainfo.xml"
	install -d "$(DESTDIR)$(UNITDIR)"
	sed 's|@BINDIR@|$(BINDIR)|g' systemd/stayproductive.service.in \
		> "$(DESTDIR)$(UNITDIR)/stayproductive.service"
	chmod 644 "$(DESTDIR)$(UNITDIR)/stayproductive.service"
	install -d "$(DESTDIR)$(EXTDIR)/gnome-extension"
	cp -R ../integrations/gnome/stayproductive@hassanraza.ai/. \
		"$(DESTDIR)$(EXTDIR)/gnome-extension/"
	install -d "$(DESTDIR)$(EXTDIR)/browser-extension"
	cp -R ../integrations/browser-extension/README.md ../integrations/browser-extension/install.py \
		../integrations/browser-extension/extension ../integrations/browser-extension/native \
		"$(DESTDIR)$(EXTDIR)/browser-extension/"
	find "$(DESTDIR)$(EXTDIR)/browser-extension" -name '__pycache__' -type d -prune -exec rm -rf {} +
	install -d "$(DESTDIR)$(DOCDIR)"
	install -m644 ../LICENSE ../NOTICE ../README.md "$(DESTDIR)$(DOCDIR)/"
	install -m644 README.md "$(DESTDIR)$(DOCDIR)/packaging.md"

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/stayproductive-daemon" \
		"$(DESTDIR)$(BINDIR)/stayproductive" \
		"$(DESTDIR)$(BINDIR)/stayproductive-x11" \
		"$(DESTDIR)$(APPDIR)/io.github.stayproductive.StayProductive.desktop" \
		"$(DESTDIR)$(ICONDIR)/io.github.stayproductive.StayProductive.svg" \
		"$(DESTDIR)$(METAINFODIR)/io.github.stayproductive.StayProductive.metainfo.xml" \
		"$(DESTDIR)$(UNITDIR)/stayproductive.service"
	rm -rf "$(DESTDIR)$(LIBDIR)" "$(DESTDIR)$(EXTDIR)" "$(DESTDIR)$(DOCDIR)"

check:
	desktop-file-validate desktop/io.github.stayproductive.StayProductive.desktop
	appstreamcli validate --no-net metainfo/io.github.stayproductive.StayProductive.metainfo.xml
	python3 -m unittest discover -s tests -v

clean:
	rm -rf out
