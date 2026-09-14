# Contributing to Banshee

- How to build Banshee, check a change, and open a pull request.
- Banshee values simplicity: the smallest change that works.
- No speculative abstractions, no new dependencies for what a few lines can
  do.

## Architecture

| Crate | Role |
| --- | --- |
| `banshee` (in `bansheed/`) | The daemon: audio capture, VAD, STT, hotkeys, JSON-RPC API. Also builds `banshee-mcp-shim` (`src/bin/`), the MCP stdio to daemon bridge |
| `banshee-common` | Shared protocol types (JSON-RPC, errors, config) |
| `banshee-app` | The desktop window: a Tauri app that is a client of the daemon's socket, shipped inside `Banshee.app` |

- The daemon exposes a JSON-RPC 2.0 API over a Unix socket at
  `~/.banshee/banshee.sock`.
- The CLI and the MCP shim are both just clients of it.

## Build

- You need stable Rust ([rustup](https://rustup.rs)).
- `banshee-app` (the desktop window) is a Tauri app that needs GTK and
  webkit2gtk.
- A plain `cargo build` builds the whole workspace, so it fails on Linux
  without those libraries.
- See [docs/linux.md](docs/linux.md) for the package names.

- On macOS:

```bash
git clone https://github.com/yamanahlawat/banshee.git
cd banshee
cargo build    # Metal + CoreML acceleration for Whisper enabled automatically
```

- On Linux, exclude `banshee-app`:

```bash
git clone https://github.com/yamanahlawat/banshee.git
cd banshee
cargo build --release --workspace --exclude banshee-app
```

- This produces `target/release/banshee` and
  `target/release/banshee-mcp-shim`.
- `banshee-tray` also builds; `banshee tray` runs it. See
  [docs/linux.md](docs/linux.md).
- An `nvidia` feature adds CUDA acceleration: add `--features nvidia`.

- Use `make install` instead to link the binaries onto your `PATH` and
  register the `systemd --user` service.
- It carries on when no systemd user bus answers. See "Install on Linux"
  below.

- The VAD tests read `silero_vad.onnx` from `~/.banshee/models/`; run
  `banshee setup` once before the suite.
- CI downloads the same file in its own step.

- Before opening a PR, make sure both of these pass:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

- On Linux, exclude `banshee-app` from both, since it needs GTK and
  WebKitGTK:

```bash
cargo test --workspace --exclude banshee-app
cargo clippy --workspace --all-targets --exclude banshee-app -- -D warnings
```

- CI runs the same clippy command on Linux, so a Linux-only warning fails the
  build there.

- For a change under `banshee-app/ui`, run the window's own checks from
  that directory.
- CI runs the same five:

```bash
npm run lint
npm run format:check
npm run check
npm test
npm run check:bundle
```

## Run your build

- Download the models once with `cargo run -p banshee -- setup` (into
  `~/.banshee/models/`).
- Then run the daemon in the foreground:

```bash
cargo run -p banshee -- serve
```

- Only one daemon can hold the socket.
- A copy already running in a terminal keeps it; an installed service
  cannot take over until you stop that copy.

## Keep the macOS grants across rebuilds

- Banshee needs Microphone and Accessibility grants; macOS ties each grant
  to the binary's code signature.
- Debug builds are ad-hoc signed, so every rebuild gets a new signature and
  drops the grants silently.
- Hotkeys stop working, with no error anywhere.

- The fix is a stable self-signed identity, set up once:

1. Open **Keychain Access** > Certificate Assistant > **Create a
   Certificate**. Name: `banshee-dev`, Identity Type: Self-Signed Root,
   Certificate Type: **Code Signing**.
2. Double-click the new certificate in the login keychain, expand **Trust**,
   and set "When using this certificate" to **Always Trust**. Without this,
   the identity exists but is not valid for signing.
3. Verify: `security find-identity -p codesigning -v` should list
   `banshee-dev`.

- `make install` needs three more things.
- Install the Tauri CLI: `cargo install tauri-cli --version "^2" --locked`.
- Install Node 22.
- Run `npm ci` in `banshee-app/ui`; the Tauri build runs `npm run build`
  there.

- From then on, build and install with:

```bash
make install
```

- It builds the release binaries and signs `/Applications/Banshee.app` with
  `banshee-dev`.
- It symlinks `banshee` and `banshee-mcp-shim` from `~/.cargo/bin` into the
  bundle, then starts the daemon and the tray launch agents.
- The bundled binaries sign under `com.banshee.app`, a different identifier
  from the binary's own, so macOS asks for Accessibility once more.
- A grant only applies to a freshly started process: approve the prompt,
  run `banshee start` again, and repeat until the hotkey earcon plays.
- After that, the signature never changes and the grant sticks.

## Install on Linux

```bash
make install
```

- Builds `banshee` and `banshee-mcp-shim` (release, `banshee-app`
  excluded), and symlinks both into `~/.local/bin` (override with
  `BIN_DIR=...`).
- Runs `banshee start`, which registers and restarts a `systemd --user`
  unit. See `bansheed/src/service.rs`.
- A machine with no systemd user bus fails that step; the install still
  finishes and names `banshee serve` as the fallback.
- Re-running `make install` after a change rebuilds and restarts the
  service.

- `make install-window` adds the desktop window. See
  [docs/linux.md](docs/linux.md).
- There is no signing step.
- `banshee tray` puts the mark in the bar.

## Submit a change

- Branch from `develop` and open your PR against it; `main` tracks releases.
- Keep commit messages to a short one-liner in the existing style:
  `feat: ...`, `fix: ...`, `chore: ...`.
- If your change is user-visible, update README.md and CHANGELOG.md with it.
