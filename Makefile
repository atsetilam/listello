# Build and install listello. The Arch package and the .deb both call `install` from here, so
# the list of files lives in one place.
#
#   make
#   sudo make install
#
# The binaries, units and desktop files name /usr/bin, so this is meant to be installed
# under /usr and nowhere else. DESTDIR is for packaging.
#
# Installing turns nothing on, for anyone. Starting listello is a separate step; see
# docs/INSTALL.md.

DESTDIR ?=
LICENSEDIR ?= /usr/share/licenses/listello
CARGO ?= cargo
CARGO_TARGET_DIR ?= $(CURDIR)/target
CARGO_FLAGS ?= --release --locked

export CARGO_TARGET_DIR

BIN = $(CARGO_TARGET_DIR)/release

.PHONY: all fetch build install

all: build

fetch:
	$(CARGO) fetch --locked --target "$$(rustc -vV | sed -n 's/^host: //p')"

build:
	$(CARGO) build $(CARGO_FLAGS) -p listello-core -p listello-agent -p listello-settings

install:
	install -Dm755 $(BIN)/listello-core $(DESTDIR)/usr/bin/listello-core
	install -Dm755 $(BIN)/listello-agent $(DESTDIR)/usr/bin/listello-agent
	install -Dm755 $(BIN)/listello-settings $(DESTDIR)/usr/bin/listello-settings

	install -d $(DESTDIR)/usr/share/listello
	install -m644 share/listello/*.svg share/listello/config.toml \
		share/listello/config.example-dock.toml $(DESTDIR)/usr/share/listello/

	install -Dm644 share/applications/org.listello.Agent.desktop \
		$(DESTDIR)/usr/share/applications/org.listello.Agent.desktop
	install -Dm644 share/applications/org.listello.Settings.desktop \
		$(DESTDIR)/usr/share/applications/org.listello.Settings.desktop
	install -Dm644 share/icons/listello.svg \
		$(DESTDIR)/usr/share/icons/hicolor/scalable/apps/listello.svg

	install -Dm644 etc/systemd/system/listello-core.service \
		$(DESTDIR)/usr/lib/systemd/system/listello-core.service
	install -Dm644 etc/systemd/system/listello-core.socket \
		$(DESTDIR)/usr/lib/systemd/system/listello-core.socket
	for unit in etc/systemd/system/systemd-backlight@*.service; do \
		install -Dm644 "$$unit" $(DESTDIR)/usr/lib/systemd/system/$$(basename "$$unit"); \
	done
	install -Dm644 etc/systemd/user/listello-agent.service \
		$(DESTDIR)/usr/lib/systemd/user/listello-agent.service
	install -Dm644 etc/udev/rules.d/99-touchbar-listello.rules \
		$(DESTDIR)/usr/lib/udev/rules.d/99-touchbar-listello.rules
	install -Dm644 etc/udev/rules.d/99-touchbar-seat.rules \
		$(DESTDIR)/usr/lib/udev/rules.d/99-touchbar-seat.rules

	install -Dm644 LICENSE $(DESTDIR)$(LICENSEDIR)/LICENSE
	install -Dm644 LICENSE.material $(DESTDIR)$(LICENSEDIR)/LICENSE.material
	install -Dm644 share/listello/LICENSE.lucide $(DESTDIR)$(LICENSEDIR)/LICENSE.lucide
