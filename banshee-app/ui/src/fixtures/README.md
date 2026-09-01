# Fixtures

Real daemon replies, captured from Banshee 0.11.1 on macOS 25.6.0: 2026-08-27,
and `permissions.json` again on 2026-09-01. Nothing here is hand-typed except
`not-running.json`, which the daemon cannot produce because a stopped daemon
answers nothing.

| File | What it is | How it was captured |
| --- | --- | --- |
| `ready.json` | `banshee.status` on a clear machine | `banshee status --json` |
| `permissions.json` | `banshee.status` with the one permission blocker a daemon can report | `tccutil reset Accessibility com.banshee.app` and the same for `ListenEvent`, daemon restarted, `banshee status --json`, Accessibility granted again |
| `pending-cues.json` | `banshee.status` with a key accepted but not applied | `banshee config set audio.cues.enabled false`, `banshee status --json`, value set back to `true` |
| `not-running.json` | what the window sees with no daemon | Constructed as `{"running": false}`, per the plan. A stopped daemon sends nothing at all, so there is no reply to record |
| `recording.json` | `banshee.state_changed` params | Subscribed to `state`, then `banshee record start` |
| `transcribing.json` | `banshee.state_changed` params | Same subscription, after `banshee record stop` |
| `speaking.json` | `banshee.state_changed` params | Same subscription, during `banshee speak` |
| `armed.json` | `banshee.state_changed` params | Same subscription, while an agent held the microphone open through `ask_user` |

## Two of these carry the discriminating case

`armed.json` carries `recording: true` beside `armed: true`, because the daemon
holds the microphone open while armed. A hand-written fixture with
`recording: false` would let a wrong branch order in `stateWord` pass its test.

`permissions.json` carries **one** blocker, which is every grant the daemon
asks for. Its `fix` is a settings path (`grant it in System Settings > Privacy &
Security > Accessibility`), never a terminal command. No `banshee permissions`
subcommand exists. A second permission blocker cannot arrive, so `fixGroups`
keeping each grant on its own row is no longer covered: `daemon.test.ts` states
only that a grant does not join the models' row, which is the pair a first run
really holds.

## Not captured

`downloading.json` is absent. No test reads it, and the Downloading state is
driven by the live `daemon:downloads` event rather than a status reply, so a
recorded status would not exercise that path. Manufacturing one meant deleting
a model already on disk.
