# What `banshee` and `banshee-mcp-shim` on the PATH point at. macOS kills a
# quarantined binary with no message at all, so a person whose Banshee arrived
# through Homebrew sees a command that exits and says nothing. This says what
# happened instead, and offers to clear the flag when a person is there to ask.
QUARANTINE_WRAPPER = <<~SH.freeze
  #!/bin/sh
  app="/Applications/Banshee.app"
  if xattr -p com.apple.quarantine "$app" >/dev/null 2>&1; then
      echo "Banshee is quarantined by macOS, so it cannot start." >&2
      echo "Homebrew marks every download, including every upgrade." >&2
      # Only a person can answer, so only a person is ever asked. A hook, an
      # agent or the launch agent has no terminal and gets the line to run.
      if [ -t 0 ] && [ -t 2 ]; then
          printf 'Clear the flag for this app now? [y/N] ' >&2
          read -r answer
          # A terminal can end the line with a carriage return.
          answer=$(printf '%s' "$answer" | tr -d '\\r')
          case "$answer" in
          [Yy] | [Yy][Ee][Ss]) xattr -dr com.apple.quarantine "$app" ;;
          *)
              echo "Left alone. To clear it: xattr -dr com.apple.quarantine $app" >&2
              exit 1
              ;;
          esac
      else
          echo "To clear it: xattr -dr com.apple.quarantine $app" >&2
          exit 1
      fi
  fi
  exec "$app/Contents/MacOS/@BINARY@" "$@"
SH

cask "banshee" do
  version "@VERSION@"
  sha256 "@SHA256@"

  url "https://github.com/yamanahlawat/banshee/releases/download/v#{version}/Banshee.app.tar.gz"
  name "Banshee"
  desc "Offline voice for coding agents, and system-wide dictation"
  homepage "https://github.com/yamanahlawat/banshee"

  depends_on arch: :arm64
  depends_on macos: :ventura

  app "Banshee.app"

  # Written here rather than shipped inside the app, and this is the whole
  # reason: macOS kills anything executed from a quarantined bundle, a shell
  # script included, so a wrapper that explains the silence has to live outside
  # it. A file written here carries no quarantine flag and always runs. It
  # clears nothing on its own.
  postflight_steps do
    write_file "bin/banshee", QUARANTINE_WRAPPER.sub("@BINARY@", "banshee"),
               base: :homebrew_prefix
    set_permissions "bin/banshee", "0755", base: :homebrew_prefix
    write_file "bin/banshee-mcp-shim", QUARANTINE_WRAPPER.sub("@BINARY@", "banshee-mcp-shim"),
               base: :homebrew_prefix
    set_permissions "bin/banshee-mcp-shim", "0755", base: :homebrew_prefix
  end

  # `remove` asks for no password; `delete` would.
  uninstall_postflight_steps do
    remove ["bin/banshee", "bin/banshee-mcp-shim"], base: :homebrew_prefix
  end

  # The formula is the same daemon without the window, and two copies fight for one socket;
  # the shim formula is an old one still in the tap. Casks can conflict only with casks.
  preflight_steps do
    if_path_exists "Cellar/banshee", base: :homebrew_prefix do
      run "/bin/sh", args: [
        "-c",
        "echo 'the banshee formula is installed; run `brew uninstall banshee` first, " \
        "then install the cask' >&2; exit 1",
      ]
    end
    if_path_exists "Cellar/banshee-mcp-shim", base: :homebrew_prefix do
      run "/bin/sh", args: [
        "-c",
        "echo 'the banshee-mcp-shim formula is installed; run `brew uninstall banshee-mcp-shim` first, " \
        "then install the cask' >&2; exit 1",
      ]
    end
  end

  uninstall launchctl: [
              "com.banshee.daemon",
              "com.banshee.tray",
            ],
            quit:      "com.banshee.app"

  zap trash: "~/.banshee"

  caveats do
    <<~EOS
      Banshee is signed but not yet notarised, so macOS refuses the downloaded app.
      Clear the flag, and again after each upgrade, because Homebrew marks every
      download:
        xattr -dr com.apple.quarantine #{appdir}/Banshee.app
      The banshee command offers to do it for you the next time you run it.
    EOS
  end
end
