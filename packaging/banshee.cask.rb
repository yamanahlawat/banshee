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

  # Both point at one script, which reads the name it was called by. macOS kills
  # a quarantined binary with no message, and Homebrew marks every download.
  binary "#{appdir}/Banshee.app/Contents/Resources/banshee-run", target: "banshee"
  binary "#{appdir}/Banshee.app/Contents/Resources/banshee-run", target: "banshee-mcp-shim"

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
