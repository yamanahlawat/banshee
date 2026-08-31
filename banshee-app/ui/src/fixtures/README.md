# Fixtures

Real daemon replies, captured 2026-08-27 from Banshee 0.11.1 on macOS 25.6.0.
Nothing here is hand-typed except `not-running.json`, which the daemon cannot
produce because a stopped daemon answers nothing.

| File | What it is | How it was captured |
| --- | --- | --- |
| `ready.json` | `banshee.status` on a clear machine | `banshee status --json` |
| `permissions.json` | `banshee.status` with two permission blockers | Banshee switched off in System Settings > Privacy & Security > Accessibility, daemon restarted, `banshee status --json`, grant restored |
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

`permissions.json` carries **two** blockers, not one. Revoking Accessibility
invalidates Input Monitoring as well. Each blocker's `fix` is a settings path
(`grant it in System Settings > Privacy & Security > Accessibility`), never a
terminal command. No `banshee permissions` subcommand exists.

## Not captured

`downloading.json` is absent. No test reads it, and the Downloading state is
driven by the live `daemon:downloads` event rather than a status reply, so a
recorded status would not exercise that path. Manufacturing one meant deleting
a model already on disk.
