# Roadmap

Banshee gives your coding agents a voice: they say what they did, ask out loud, and you
answer by talking, across every agent you run. It also types what you dictate into any app.
This page holds what lands next and what is ruled out. `BACKLOG.md` holds what is wrong
today, and `CHANGELOG.md` holds what has shipped.

## Next

1. **Remote URLs need HTTPS.** Plain HTTP stays allowed to this machine and your own network.
   It is first because today audio and text can cross a network unencrypted.
2. **Cut what the loop doesn't need, and define each setting once.** `banshee tell`,
   `listen_for_prompt` with the keep key and `banshee listen`, the system-voice fallback, and
   `service uninstall` and `tray --uninstall`, which `banshee uninstall` already covers. Each
   remaining setting is defined once in `banshee-common`, and both `banshee config set` and
   the window read that list, so the two always offer the same settings. It comes before the
   items below because they add settings.
3. **Try transcribe.cpp.** Swap whisper.cpp for it and add streaming Parakeet Unified next
   to the Whisper presets. Measure error rate and time to text on real recordings first,
   and switch only if it wins. The measurement can start now; the swap waits for item 2.
4. **Every microphone use names its client.** When the microphone opens, Banshee names the
   program that asked for it, and the log records it. The daemon reads the name of the
   connected process from the operating system. A process can still rename itself or run a
   renamed copy of a program, so the name is a lead to check. For the MCP shim and the
   turn-end hooks, the daemon reads the parent process. What each agent's parent shows isn't
   measured yet. The capsule shows it on macOS. Where Linux shows it is still open. Any
   process running as you can still reach the socket and read `banshee.db`. This item makes
   each use visible. It comes after the cut, so fewer commands open the microphone.
5. **`SMAppService` for the daemon and the tray.** Banshee stops writing launchd plists and
   calling `launchctl`. It works with a self-signed app, even one whose certificate the Mac
   doesn't trust: tested on macOS 14 and 26 in
   [smappservice-spike](https://github.com/yamanahlawat/smappservice-spike). A login on a real
   Mac still has to confirm the agent starts again. It comes after the cut, which removes the
   service code it would otherwise replace.
6. **Linux packaging:** an AppImage and an AUR package, so the window and the tray install
   without a source clone. It comes before notarization because it can start today.
7. **Notarization,** once a Developer ID is affordable. SentinelOne doesn't flag Handy,
   which is notarized and starts at login through `SMAppService`. Which of the two stops the
   flag isn't measured. Check the flag once item 5 ships, before paying for a Developer ID.

## Community sized

- More agents for `banshee connect`: Windsurf (#55), Zed (#56) and Kiro (#58). Cline and Roo
  Code, Goose and Continue have no issue yet.
- Confirm `banshee connect cursor` (#53) and `banshee connect codex` (#54) on a real install.

## Not planned

- An `ask_user` that waits for a key press. The loop depends on the microphone opening by
  itself.
- Desktop control through an agent, which was `banshee tell`. Open the agent instead.
- Cloud by default. Nothing leaves the machine unless you set a remote server.
