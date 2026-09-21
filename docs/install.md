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
- **Update it** with `brew upgrade --cask banshee`.
- **Remove it** with `brew uninstall --cask banshee`, and add `--zap` to take
  `~/.banshee` as well.
- **The `banshee` command** is on your `PATH` as well as in the app.

## macOS, without Homebrew

```bash
curl -fsSL https://github.com/yamanahlawat/banshee/releases/latest/download/Banshee.app.tar.gz \
  | tar -xzf - -C /Applications
```

- **The unpacked app** sets no quarantine flag, so it needs no `xattr` line.
- **Opening the app** puts the `banshee` command on your `PATH`, at
  `/usr/local/bin/banshee`. That directory is first in `/etc/paths`, so it
  answers in every terminal. `banshee start` does the same from a terminal.
- **A clean macOS has no `/usr/local/bin`,** and its parent belongs to `root`,
  so Banshee cannot make it. One line does:

```bash
sudo mkdir -p /usr/local/bin && sudo ln -s \
  /Applications/Banshee.app/Contents/MacOS/banshee /usr/local/bin/banshee
```

- **Nothing of yours is replaced.** A `banshee` that already answers is left
  alone, so a Homebrew install keeps the wrapper that clears its quarantine
  flag. There is no `-f` above, so a file or a link of your own at that name
  stays, and `ln` says `File exists` rather than writing over it.
- **The command inside the app** stays at
  `/Applications/Banshee.app/Contents/MacOS/banshee`, whether it is linked or
  not.
- **No admin account?** Unpack into `~/Applications`, and read that path
  instead.
- **Update it** by running the same command again: it replaces the app in place.
- **Remove it** with `banshee uninstall`, which stops the daemon and takes the
  login entries and the link with it.

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

- **You get** the daemon, the `banshee` command and `banshee-update`.
- **`banshee watch --waybar`** feeds a status bar.
- **The typing tool and the service** are in [linux.md](linux.md).
- **Update it** with `banshee-update`, which fetches the latest release.
- **Remove it** with `banshee uninstall`.

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

```bash
banshee uninstall
```

- **It stops Banshee** and takes it out of login, whichever way it was installed.
- **It shows what it will remove** and asks before it removes anything.
- **Homebrew's copy stays Homebrew's:** the command names
  `brew uninstall --cask banshee` rather than deleting files Homebrew records.
- **A source build stays:** `make install` records nothing, so the command
  names the binary it runs from and leaves every file the install placed.
- **`--data`** also deletes `~/.banshee`: the models, the history and the keys.
  Without it, they stay.
- **`--yes`** removes without asking, for a script.
- **The cask alone:** `brew uninstall --zap --cask banshee` does the same in one
  step, and `--zap` takes `~/.banshee` with it.
