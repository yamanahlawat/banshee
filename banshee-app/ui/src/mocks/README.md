# Mocks

`bridge.ts` answers the window's calls when Tauri is absent, and `?state=`
picks the reply to answer with. It imports `ready.json`, `permissions.json`,
`not-running.json`, `remote.json` and `remote-speech.json`, and spreads `ready`
for the states it builds itself. `daemon.test.ts` reads the other five files.

Real daemon replies, captured on macOS 25.6.0: Banshee 0.11.1 on 2026-08-27,
`permissions.json` again on 2026-09-01, `remote.json` from Banshee 0.12.2 on
2026-09-09, and `remote-speech.json` on 2026-09-10. Nothing here is hand-typed
except `not-running.json`, which the
daemon cannot produce because a stopped daemon answers nothing, and two objects
in `remote.json` that the capture predates: `remote.tts` and
`config.tts.remote`. `remote-speech.json` holds both of those from a real
reply, so it is the one to read for the speaker's shape.

| File | What it is | How it was captured |
| --- | --- | --- |
| `ready.json` | `banshee.status` on a clear machine | `banshee status --json` |
| `permissions.json` | `banshee.status` with the one permission blocker a daemon can report | `tccutil reset Accessibility com.banshee.app` and the same for `ListenEvent`, daemon restarted, `banshee status --json`, Accessibility granted again |
| `pending-cues.json` | `banshee.status` with a key accepted but not applied | `banshee config set audio.cues.enabled false`, `banshee status --json`, value set back to `true` |
| `not-running.json` | what the window sees with no daemon | Constructed, not captured |
| `recording.json` | `banshee.state_changed` params | Subscribed to `state`, then `banshee record start` |
| `transcribing.json` | `banshee.state_changed` params | Same subscription, after `banshee record stop` |
| `speaking.json` | `banshee.state_changed` params | Same subscription, during `banshee speak` |
| `armed.json` | `banshee.state_changed` params | Same subscription, while an agent held the microphone open through `ask_user` |
| `remote.json` | `banshee.status` with a remote listener set and its key present | `banshee config set stt.provider remote`, `stt.remote.model` set to `gpt-4o-transcribe`, `base_url` left at the OpenAI default so no private host enters the repo, the key set, daemon restarted, `banshee status --json`. The `remote.tts` object and the `config.tts.remote` table were filled in by hand on 2026-09-10, with the values the daemon answers for a local speaker. `the_window_mocks_carry_every_config_key_the_reply_writes` in `bansheed/src/api/tests.rs` holds the `config` half to the shape the daemon serialises |
| `remote-speech.json` | `banshee.status` with a remote listener and a remote speaker, both keys present and the speaker started | `banshee config remote` for both sides, `tts.remote.base_url` left at the OpenAI default so no private host enters the repo, daemon restarted, `banshee status --json` |

## Two of these carry the discriminating case

`armed.json` carries `recording: true` beside `armed: true`, because the daemon
holds the microphone open while armed. A hand-written mock with
`recording: false` would let a wrong branch order in `stateWord` pass its test.

`permissions.json` carries **one** blocker, which is every grant the daemon
asks for. Its `fix` is a settings path (`grant it in System Settings > Privacy &
Security > Accessibility`), never a terminal command. No `banshee permissions`
subcommand exists. A second permission blocker cannot arrive, so no test covers
`fixGroups` with two grants on two rows. `daemon.test.ts` states only that a
grant does not join the models' row, which is the pair a first run really
holds.

## Not captured

`downloading.json` is absent. No test reads it, and the Downloading state is
driven by the live `daemon:downloads` event rather than a status reply, so a
recorded status would not exercise that path. Manufacturing one meant deleting
a model already on disk.
