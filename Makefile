# Install Banshee, one target that branches by OS.
#
# macOS: a signed app bundle, so macOS draws its icon and TCC grants survive
# rebuilds. One-time setup: Keychain Access > Certificate Assistant > Create
# a Certificate, name "banshee-dev", type "Code Signing", self-signed. Needs:
# a Rust toolchain, `cargo install tauri-cli`, and Node 22. The Tauri build
# runs `npm run build` in banshee-app/ui, so run `npm ci` there first.
# /Applications needs an admin account; set APP_DIR=$HOME/Applications
# without one.
#
# Linux: `install` builds and installs the daemon and the CLI. No signing, no
# bundle: build, symlink onto PATH, and register+start the systemd --user
# service. `install-window` adds the desktop window. It needs GTK, WebKitGTK
# and Node.
UNAME_S := $(shell uname -s)

IDENTITY ?= banshee-dev
APP_DIR ?= /Applications
APP := $(APP_DIR)/Banshee.app
BANSHEE := $(APP)/Contents/MacOS/banshee
VERSION := $(shell grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

.PHONY: install

ifeq ($(UNAME_S),Darwin)

BIN_DIR ?= $(HOME)/.cargo/bin

install:
	cargo build --release --workspace --exclude banshee-app
	# Last, and never followed by a plain `cargo build`: that rebuilds the app
	# without the frontend and leaves it pointing at the dev server.
	cd banshee-app && cargo tauri build --no-bundle
	mkdir -p "$(APP_DIR)" "$(BIN_DIR)"
	./scripts/bundle.sh target/release "$(APP)" "$(IDENTITY)" "$(VERSION)"
	ln -sf "$(BANSHEE)" "$(BIN_DIR)/banshee"
	ln -sf "$(APP)/Contents/MacOS/banshee-mcp-shim" "$(BIN_DIR)/banshee-mcp-shim"
	"$(BANSHEE)" start
	"$(BANSHEE)" tray
	rm -f "$(BIN_DIR)/banshee-tray"

else

# ~/.local/bin, not ~/.cargo/bin: it's what a `systemd --user` service and
# non-interactive shells see, and ~/.cargo/bin often isn't.
BIN_DIR ?= $(HOME)/.local/bin
DESKTOP_DIR ?= $(HOME)/.local/share/applications
ICON_DIR ?= $(HOME)/.local/share/icons/hicolor

install:
	cargo build --release --workspace --exclude banshee-app
	mkdir -p "$(BIN_DIR)"
	ln -sf "$(CURDIR)/target/release/banshee" "$(BIN_DIR)/banshee"
	ln -sf "$(CURDIR)/target/release/banshee-mcp-shim" "$(BIN_DIR)/banshee-mcp-shim"
	# A machine with no user systemd bus fails this step. The install must not
	# abort here, since the symlinks above already point at a working build.
	"$(BIN_DIR)/banshee" start || echo "could not register the systemd --user service; start it yourself with: $(BIN_DIR)/banshee serve"
	@echo "installed to $(BIN_DIR); make sure it's on your PATH"

.PHONY: install-window

# Depends on `install`, so the daemon exists in target/release before the
# window is installed. It needs GTK, WebKitGTK and Node, which `install` does
# not, so a headless machine keeps its one-command install.
install-window: install
	cd banshee-app/ui && npm ci
	cd banshee-app && cargo tauri build --no-bundle
	mkdir -p "$(BIN_DIR)" "$(DESKTOP_DIR)" "$(ICON_DIR)/512x512/apps"
	# Canonicalised at run time, so the window finds `banshee` in target/release.
	ln -sf "$(CURDIR)/target/release/banshee-app" "$(BIN_DIR)/banshee-app"
	cp assets/banshee-icon.png "$(ICON_DIR)/512x512/apps/banshee.png"
	sed -e 's|@BIN_DIR@|$(BIN_DIR)|' packaging/banshee.desktop > "$(DESKTOP_DIR)/banshee.desktop"
	update-desktop-database "$(DESKTOP_DIR)" 2>/dev/null || true
	@echo "installed the window; open Banshee from your launcher"

endif
