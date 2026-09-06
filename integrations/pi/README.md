# Pi coding agent

An extension that gives the [Pi coding agent](https://github.com/earendil-works/pi-coding-agent)
the same three voice tools the MCP shim exposes: `speak_status`, `ask_user`,
and `listen_for_prompt`.

Pi has its own extension API, so it talks to the Banshee daemon directly over
`~/.banshee/banshee.sock` rather than going through `banshee-mcp-shim`. Nothing
extra to install or configure.

## Demo

<https://github.com/user-attachments/assets/006132bd-9710-4322-a35a-4a5e5004371c>

The daemon running with Pi. It asks which language to use, hears "let's go with
python", and writes the file. Nothing was typed.

## Install

```bash
banshee connect pi
```

That writes `~/.pi/agent/extensions/banshee.ts` after showing you the change. Without the
`banshee` CLI, fetch the file yourself:

```bash
mkdir -p ~/.pi/agent/extensions
curl -o ~/.pi/agent/extensions/banshee.ts \
  https://raw.githubusercontent.com/yamanahlawat/banshee/main/integrations/pi/banshee.ts
```

Restart Pi. Extensions in that directory need no registration.

The daemon has to be running, with the models already downloaded:

```bash
banshee setup   # once; downloads the models
banshee start
```

If it isn't, every tool call fails with `Banshee is not running. Start it with:
banshee start` rather than a stack trace.

## What you get

| Tool | What the agent does with it |
| --- | --- |
| `speak_status` | Say something aloud, for decisions made and work finished |
| `ask_user` | Ask a question aloud, then wait for and return your spoken answer |
| `listen_for_prompt` | Pick up anything you've said since it last checked |

All three run in `sequential` mode. The microphone and speaker are one device
and the daemon rejects overlapping sessions, so letting Pi call them in
parallel would produce `-32004` errors.

The extension renders its own tool call and result rows, so you see the
question Banshee asked and the answer it heard, not just a tool name.

## Keeping it in sync

Tool descriptions are duplicated between this file and
`bansheed/src/bin/banshee-mcp-shim.rs`. They steer how the agent splits speech from
written output, so if you change one, change the other.
