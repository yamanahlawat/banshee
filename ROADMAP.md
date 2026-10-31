# Roadmap

Banshee gives your coding agents a voice: they say what they did, ask out loud, and you
answer by talking, across every agent you run. It also types what you dictate into any app.
This page holds what lands next and what is ruled out. `BACKLOG.md` holds what is wrong
today, and `CHANGELOG.md` holds what has shipped.

## Next

1. **Mic and models on demand.** The owner's reason: "model loading is just for optimizing the
   resources usage per system - some system might use bigger models. so keep the model loaded
   always doesn't make sense." And: "the mic should also not be held always." Banshee then
   loads a model when it is needed and frees it when idle, and opens the microphone for a use
   and closes it after. Loading is cheap enough: models load in under 360 ms on the Mac and the
   Linux box. The first load after an install compiles Metal shaders and took 2.7 to 3.1 s,
   so an on-demand load should not be the first load. A Bluetooth headset is the cost. Its first sound came about 311 ms after the open,
   and its speaker drops to 1 channel at 16 kHz while the mic is open. That was measured on one
   headset and one Mac. It holds the idle unload for the speech models and the listener, a
   shorter hold or an open per use for the mic, and a check on other headsets.
2. **The tray shows a lasting problem.** A warning mark, and a first menu line that names the
   problem and opens the window where it is fixed. Failures nobody sees are the most common
   complaint about dictation and agent voice tools.
3. **Banshee checks for a text field before typing.** When nothing can take the text, it goes
   on the clipboard and the capsule says "Copied". Text typed into the wrong place is among the
   most common complaints too.
4. **Every setting applies at once, and each is defined once.** The hotkey, a switch to or
   from a server, and the answer's quiet length stop needing a restart. Each setting is then
   defined once in `banshee-common`, and both `banshee config set` and the window read that list.
   It comes after the items that change settings, and before the window is rebuilt on it.
5. **History keeps questions and their audio.** The window gets a sidebar, and History shows a
   list with the selected row beside it. An agent's question and your spoken answer are kept
   as a pair. Audio stays for 7 days by default, and a failed transcription can be retried
   from it.
6. **The tray switches the listening model and the voice,** with no restart.
7. **A capsule on Linux, then the answer countdown on both platforms.** Linux has no capsule
   today, and the countdown needs it.
8. **End-of-turn detection with Smart Turn,** so a thinking pause doesn't end an answer. On 500
   held-out clips at threshold 0.2, it waited out 92.4% of unfinished pauses and held 3.2% of
   finished answers past 2.5 s. Each check took 13 ms on the Mac and 40 ms on the Ryzen. How
   long to wait after an unfinished pause isn't set yet.
9. **Every microphone use names its client.** When the microphone opens, Banshee names the
    program that asked for it, and the log records it. The daemon reads the name of the
    connected process from the operating system. A process can still rename itself or run a
    renamed copy of a program, so the name is a lead to check. For the MCP shim and the
    turn-end hooks, the daemon reads the parent process. What each agent's parent shows isn't
    measured yet. The capsule shows it on macOS. Where Linux shows it is still open. Any
    process running as you can still reach the socket and read `banshee.db`. This item makes
    each use visible.
10. **Linux packaging:** an AppImage and an AUR package, so the window and the tray install
    without a source clone. Release builds already target AVX2 on x86_64 and plain armv8-a on
    aarch64. A GPU build (Vulkan, CUDA or ROCm) is tried
    here; none is measured yet. It comes before notarization because it can start today.
11. **Notarization,** once a Developer ID is affordable. SentinelOne doesn't flag Handy,
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
