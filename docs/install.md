# Installing Banshee

- **The README** shows the shortest route on each platform.
- **This page** holds every route, and how to remove each one.
- **Disk:** every route needs ~860 MB for the models.
- **Intel Macs** are not supported.

## macOS, with the window

```bash
brew install --cask yamanahlawat/banshee/banshee
xattr -dr com.apple.quarantine /Applications/Banshee.app
```

- **Homebrew** marks every download as quarantined, including every upgrade.
- **A quarantined Banshee** does not open, and macOS kills its `banshee` command.
- **The `xattr` line** clears that flag.
- **The `banshee` command** offers to clear it for you when you next run it in a
  terminal. It asks first, and only ever asks a person.
- **The `banshee` command** is on your `PATH` as well as in the app.

## macOS, without Homebrew

```bash
curl -fsSL https://github.com/yamanahlawat/banshee/releases/latest/download/Banshee.app.tar.gz \
  | tar -xzf - -C /Applications
```

- **The unpacked app** sets no quarantine flag, so it needs no `xattr` line.
- **The `banshee` command** is
  `/Applications/Banshee.app/Contents/MacOS/banshee`.
- **Link it** onto your `PATH` if you want it short.
- **No admin account?** Unpack into `~/Applications`, and read that path
  instead.

## macOS, terminal only

```bash
brew install --formula yamanahlawat/banshee/banshee
```

- **You get** the daemon, the `banshee` command and the menu bar icon.
- **To add the window,** remove the formula first ([Uninstall](#uninstall)),
  then install the cask.

## Linux, with Homebrew

```bash
brew install --formula yamanahlawat/banshee/banshee
```

Or without Homebrew:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/yamanahlawat/banshee/releases/latest/download/banshee-installer.sh | sh
```

- **You get** the daemon and the `banshee` command.
- **`banshee watch --waybar`** feeds a status bar.
- **The typing tool and the service** are in [linux.md](linux.md).

## From source

- **Clone the repo** and run `make install`. See
  [CONTRIBUTING.md](../CONTRIBUTING.md).
- **It works** on macOS and Linux.
- **On Linux** it also registers and starts the `systemd --user` service, where
  systemd answers.
- **Without systemd** the install still finishes, and `banshee serve` starts the
  daemon.

## Set up from the terminal

- **The window** does all of this for you.
- **These steps** do the same work without it.

**1. Download the models:**

```bash
banshee setup
```

- **It fetches** what is missing: Whisper, Silero VAD, Kokoro, about 860 MB.
- **An interrupted download** resumes, and a re-run fetches only what is still missing.

**2. Start it:**

```bash
banshee start
```

- **`banshee start`** runs the daemon at once, and at every login.
- **It downloads nothing.** A model that is missing is named, and `banshee setup` fetches it.

**3. Grant the macOS permissions:**

- **Banshee needs two,** or it quietly fails to record or type.
- **Microphone** captures the audio.
- **Accessibility** serves the global hotkey and the typing.
- **macOS asks** for each one the first time Banshee needs it.
- **Approve it,** and the daemon restarts itself to pick the grant up.

**4. Check it:**

```bash
banshee status
```

- **`banshee status`** reports the models, the microphone, the permissions and
  the daemon.

## Uninstall

- **The cask:** `brew uninstall --zap --cask banshee`. `--zap` also removes
  `~/.banshee`.
- **The formula or the installer:** `banshee tray --uninstall`, then
  `banshee service uninstall`, then `brew uninstall --formula banshee`.
- **Without Homebrew,** delete the binaries by hand instead.
- **The app from `curl`:** the two `banshee` commands above, then delete
  `/Applications/Banshee.app`.
- **`~/.banshee`** holds the models, the history and `config.toml`. Delete it
  after the last two routes if you want it gone.
