# Why Banshee

Plenty of tools will transcribe your voice into an editor. Banshee is built for
the other half of the conversation.

- **Your agent asks, you answer.** `ask_user` speaks a question, waits for
  playback to finish, opens the microphone, and returns what you said, all in
  one call. Most voice tooling is one-directional dictation; this is a loop.
- **It never hears itself.** The microphone opens only after the question has
  finished playing, so the daemon can't transcribe its own voice. That's why
  Banshee works on laptop speakers without a headset.
- **Nothing leaves your machine.** Whisper, Silero VAD, and Kokoro all run
  locally. No API keys, no cloud tier, no audio uploaded, works on a plane.
- **It waits while you think.** Answers end on 2.5s of silence rather than the
  usual few hundred milliseconds, so pausing mid-sentence to think doesn't cut
  you off.
- **It handles your jargon in both directions.** `vocabulary` biases Whisper
  toward project words it would otherwise mangle, and the espeak-ng fallback
  pronounces unfamiliar terms instead of spelling them out letter by letter.
- **Not tied to one vendor.** It's an MCP server, so Claude Code, Cursor,
  OpenCode, and anything else that speaks MCP all work.
- **One daemon, both jobs.** Agent voice and system-wide dictation share the
  same process, models, and microphone.
