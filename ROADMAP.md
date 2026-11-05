# Roadmap

Banshee gives your coding agents a voice: they say what they did, ask out loud, and you
answer by talking, across every agent you run. It also types what you dictate into any app.
This page holds what lands next and what is ruled out. `BACKLOG.md` holds what is wrong
today, and `CHANGELOG.md` holds what has shipped.

## Next

1. **History keeps questions and their audio.** The window gets a sidebar, and History shows a
   list with the selected row beside it. An agent's question and your spoken answer are kept
   as a pair. Audio stays for 7 days by default, and a failed transcription can be retried
   from it.
2. **The tray switches the listening model and the voice,** with no restart.
3. **A capsule on Linux, then the answer countdown on both platforms.** Linux has no capsule
   today, and the countdown needs it.
4. **End-of-turn detection with Smart Turn,** so a thinking pause doesn't end an answer. On 500
   held-out clips at threshold 0.2, it waited out 92.4% of unfinished pauses and held 3.2% of
   finished answers past 2.5 s. Each check took 13 ms on the Mac and 40 ms on the Ryzen. How
   long to wait after an unfinished pause isn't set yet.
5. **Every microphone use names its client.** When the microphone opens, Banshee names the
   program that asked for it, and the log records it. The daemon reads the name of the
   connected process from the operating system. A process can still rename itself or run a
   renamed copy of a program, so the name is a lead to check. For the MCP shim and the
   turn-end hooks, the daemon reads the parent process. What each agent's parent shows isn't
   measured yet. The capsule shows it on macOS. Where Linux shows it is still open. Any
   process running as you can still reach the socket and read `banshee.db`. This item makes
   each use visible.
6. **Linux packaging:** an AppImage and an AUR package, so the window and the tray install
   without a source clone. Release builds already target AVX2 on x86_64 and plain armv8-a on
   aarch64. A GPU build (Vulkan, CUDA or ROCm) is tried
   here; none is measured yet. It comes before notarization because it can start today.
7. **Notarization,** once a Developer ID is affordable. SentinelOne doesn't flag Handy,
   which is notarized and starts at login through `SMAppService`. Which of the two stops the
   flag isn't measured. Check the flag once the `SMAppService` release ships, before paying
   for a Developer ID.

## Community sized

- More agents for `banshee connect`: Windsurf (#55), Zed (#56) and Kiro (#58). Cline and Roo
  Code, Goose and Continue have no issue yet.
- Confirm `banshee connect cursor` (#53) and `banshee connect codex` (#54) on a real install.

## Not planned

- An `ask_user` that waits for a key press. The loop depends on the microphone opening by
  itself.
- Desktop control through an agent, which was `banshee tell`. Open the agent instead.
- Cloud by default. Nothing leaves the machine unless you set a remote server.
