# Why Banshee

Banshee is a local voice daemon: it gives your coding agent a voice, and it
types what you dictate into any app.

- **Other tools stop at dictation.** Banshee is built for the other half of
  the conversation.
- **Your agent asks, you answer.** `ask_user` speaks a question, waits for
  playback, opens the microphone, and returns your answer, in one call.
- Most voice tooling is one-directional. This is a loop.
- **It never hears itself.** The microphone opens only after the question
  finishes playing, so the daemon cannot transcribe its own voice.
- That is why Banshee works on laptop speakers, with no headset needed.
- **Nothing leaves your machine by default.** Whisper, Silero VAD and Kokoro
  all run locally.
- No account, no cloud tier. It works on a plane.
- A remote listener and a remote voice are yours to turn on; the tray names
  it when either runs.
- **It waits while you think.** Answers end on 2.5 seconds of silence, not
  the usual few hundred milliseconds.
- A mid-sentence pause to think does not cut you off.
- **It handles your jargon in both directions.** `vocabulary` biases Whisper
  toward project words it would otherwise mangle.
- The espeak-ng fallback pronounces unfamiliar terms instead of spelling
  them out.
- **Not tied to one vendor.** It is an MCP server.
- Claude Code, Cursor, OpenCode and anything else that speaks MCP all work.
- **One daemon, both jobs.** Agent voice and dictation share the same
  process, models and microphone.
