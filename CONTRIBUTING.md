# Contributing to Banshee

Thanks for helping out! Banshee values simplicity: the smallest change that
works, no speculative abstractions, no new dependencies for what a few lines
can do.

## Building

You need stable Rust ([rustup](https://rustup.rs)) and, for now, macOS.

```bash
git clone https://github.com/yamanahlawat/banshee.git
cd banshee
cargo build    # Metal + CoreML acceleration for Whisper enabled automatically on macOS
```

Before opening a PR, make sure both of these pass:

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

## Running your build

Download the models once with `cargo run -p banshee -- setup`
(about 1 GB into `~/.banshee/models/`), then run the daemon in the foreground:

```bash
cargo run -p banshee -- serve
```

## Keeping macOS permissions across rebuilds

Banshee needs Microphone, Input Monitoring, and Accessibility grants, and
macOS ties each grant to the binary's code signature. Debug builds are ad-hoc
signed, so every rebuild gets a new signature and silently drops the grants:
hotkeys just stop working, with no error anywhere.

The fix is a stable self-signed identity, set up once:

1. Open **Keychain Access** > Certificate Assistant > **Create a
   Certificate**. Name: `banshee-dev`, Identity Type: Self-Signed Root,
   Certificate Type: **Code Signing**.
2. Double-click the new certificate in the login keychain, expand **Trust**,
   and set "When using this certificate" to **Always Trust**. Without this,
   the identity exists but is not valid for signing.
3. Verify: `security find-identity -p codesigning -v` should list
   `banshee-dev`.

From then on, build and install with:

```bash
make install
```

It installs the release binary to `~/.cargo/bin`, signs it with `banshee-dev`,
and restarts the daemon. The first signed run needs one last round of
permission grants; each grant only applies to a freshly started process, so
expect to approve a prompt, run `banshee start` again, and repeat until the
hotkey earcon plays. After that, the signature never changes and the grants
stick.

## Submitting changes

- Branch from `develop` and open your PR against it; `main` tracks releases.
- Keep commit messages to a short one-liner in the existing style:
  `feat: ...`, `fix: ...`, `chore: ...`.
- If your change is user-visible, update README.md and CHANGELOG.md with it.
