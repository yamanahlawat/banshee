# Roadmap

Banshee gives your coding agents a voice: they say what they did, ask out loud, and you
answer by talking, across every agent you run. It also types what you dictate into any app.
This page holds what lands next and what is ruled out. `BACKLOG.md` holds what is wrong
today, and `CHANGELOG.md` holds what has shipped.

## Next

1. **Cut what the loop doesn't need.** `banshee tell`, `banshee listen`, `listen_for_prompt`,
   Shift plus the hotkey, `service uninstall` and `tray --uninstall` go, so a recording always
   dictates. It comes first because the items below build on a smaller daemon.
2. **`SMAppService` for the daemon and the tray.** Banshee stops writing launchd plists and
   calling `launchctl`. It works with a self-signed app, even one whose certificate the Mac
   doesn't trust: tested on macOS 14 and 26 in
   [smappservice-spike](https://github.com/yamanahlawat/smappservice-spike). A login on a real
   Mac still has to confirm the agent starts again. It comes after the cut, which removes the
   service code it would otherwise replace.
3. **Pocket TTS with the companion voice becomes the local voice, and it speaks when a remote
   server fails.** It replaces `say` and `espeak-ng` as the fallback, which sound robotic, and
   Kokoro as the default. Kokoro's voices stay as choices. Pocket runs in FP32 at 4 decoding
   steps on `ort`. Measured in a Rust spike against Kokoro:
   - On the Mac on battery: 9.0x real time, first audio 28 ms, cold load 240 ms. In Low Power
   Mode: 6.7x, 38 ms and 331 ms. Kokoro: 8.6x, 279 ms and 321 ms.
   - On a Ryzen 9 9900X: 4.4x, first audio 50 to 66 ms, cold load 354 to 398 ms. Kokoro: 14.6x,
   92 to 511 ms and 336 to 340 ms.
   - Memory: 672 MB after load and 866 MB at peak, against Kokoro's 603 and 970 MB.

   Its files, about 399 MB, download on the first choice of the voice, and Banshee credits
   Kyutai (CC-BY-4.0). Pocket has no speed control, so `tts.speed` goes; an old config that
   holds it still loads. When the local voice can't load, Banshee stays up, says nothing, and
   `banshee status` names why. It comes after the cut because it changes `tts.fallback`.
4. **Listening moves to transcribe.cpp.** Streaming Parakeet Unified becomes the default,
   Whisper turbo the second suggestion, and every other transcribe.cpp model can be
   downloaded. Fast and Quality go. On 200 Pipecat clips, Parakeet matched Balanced's error
   rate (2.81% against 2.89%). Its median time to text was 145 ms on the Ryzen at 8 threads,
   where Balanced took 10.5 s, and 38 ms on the Mac, where Balanced took 701 ms. It also shows
   words while you speak. `whisper-rs` goes in the same change: both bundle ggml, and linked
   together the build fails on Linux and mixes the two copies on the Mac. It comes after the
   cut, beside item 3.
5. **The tray shows a lasting problem.** A warning mark, and a first menu line that names the
   problem and opens the window where it is fixed. Failures nobody sees are the most common
   complaint about dictation and agent voice tools.
6. **Banshee checks for a text field before typing.** When nothing can take the text, it goes
   on the clipboard and the capsule says "Copied". Text typed into the wrong place is among the
   most common complaints too.
7. **Every setting applies at once, and each is defined once.** The hotkey, a switch to or
   from a server, and the answer's quiet length stop needing a restart. Each setting is then
   defined once in `banshee-common`, and both `banshee config set` and the window read that list.
   It comes after the items that change settings, and before the window is rebuilt on it.
8. **History keeps questions and their audio.** The window gets a sidebar, and History shows a
   list with the selected row beside it. An agent's question and your spoken answer are kept
   as a pair. Audio stays for 7 days by default, and a failed transcription can be retried
   from it.
9. **The tray switches the listening model and the voice,** with no restart.
10. **A capsule on Linux, then the answer countdown on both platforms.** Linux has no capsule
    today, and the countdown needs it.
11. **End-of-turn detection with Smart Turn,** so a thinking pause doesn't end an answer. On 500
    held-out clips at threshold 0.2, it waited out 92.4% of unfinished pauses and held 3.2% of
    finished answers past 2.5 s. Each check took 13 ms on the Mac and 40 ms on the Ryzen. How
    long to wait after an unfinished pause isn't set yet.
12. **Every microphone use names its client.** When the microphone opens, Banshee names the
    program that asked for it, and the log records it. The daemon reads the name of the
    connected process from the operating system. A process can still rename itself or run a
    renamed copy of a program, so the name is a lead to check. For the MCP shim and the
    turn-end hooks, the daemon reads the parent process. What each agent's parent shows isn't
    measured yet. The capsule shows it on macOS. Where Linux shows it is still open. Any
    process running as you can still reach the socket and read `banshee.db`. This item makes
    each use visible. It comes after the cut, so fewer commands open the microphone.
13. **Linux packaging:** an AppImage and an AUR package, so the window and the tray install
    without a source clone. transcribe.cpp builds for the host CPU on Linux unless told not
    to, so a release build sets `GGML_NATIVE=OFF`. A GPU build (Vulkan, CUDA or ROCm) is tried
    here; none is measured yet. It comes before notarization because it can start today.
14. **Notarization,** once a Developer ID is affordable. SentinelOne doesn't flag Handy,
    which is notarized and starts at login through `SMAppService`. Which of the two stops the
    flag isn't measured. Check the flag once item 2 ships, before paying for a Developer ID.

## Community sized

- More agents for `banshee connect`: Windsurf (#55), Zed (#56) and Kiro (#58). Cline and Roo
  Code, Goose and Continue have no issue yet.
- Confirm `banshee connect cursor` (#53) and `banshee connect codex` (#54) on a real install.

## Not planned

- An `ask_user` that waits for a key press. The loop depends on the microphone opening by
  itself.
- Desktop control through an agent, which was `banshee tell`. Open the agent instead.
- Cloud by default. Nothing leaves the machine unless you set a remote server.
