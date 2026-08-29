# Install Banshee as a signed app bundle so macOS draws its icon, and so TCC
# grants survive rebuilds.
# One-time setup: Keychain Access > Certificate Assistant > Create a Certificate,
# name "banshee-dev", type "Code Signing", self-signed.
IDENTITY ?= banshee-dev
APP_DIR ?= $(HOME)/Applications
APP := $(APP_DIR)/Banshee.app
BIN_DIR ?= $(HOME)/.cargo/bin
BANSHEE := $(APP)/Contents/MacOS/banshee
VERSION := $(shell grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

.PHONY: install
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
