# Reinstall the daemon with a stable signing identity so TCC grants survive rebuilds.
# One-time setup: Keychain Access > Certificate Assistant > Create a Certificate,
# name "banshee-dev", type "Code Signing", self-signed.
IDENTITY ?= banshee-dev
CARGO_INSTALL_ROOT ?= $(HOME)/.cargo
BANSHEE := $(CARGO_INSTALL_ROOT)/bin/banshee

.PHONY: install
install:
	cargo install --path bansheed --features apple --root "$(CARGO_INSTALL_ROOT)"
	cargo install --path banshee-mcp-shim --root "$(CARGO_INSTALL_ROOT)"
	codesign --force --sign "$(IDENTITY)" "$(BANSHEE)"
	codesign --verify --strict --verbose=2 "$(BANSHEE)"
	
	"$(BANSHEE)" start
