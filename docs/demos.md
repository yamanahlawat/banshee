# The demos

The talking and each change to the desktop run in real time. The quiet stretches, where
the agent works, run faster.
White captions carry the person's words. Orange captions carry Banshee's voice.

## Linux, on Omarchy

### The desktop, set up for a recording

<https://github.com/user-attachments/assets/7ec3eaa1-33d6-4cdd-8e5b-10ae11871918>

The person asks for a screencast setup: no screen lock, no top bar, no window gaps, and
rounded corners. Claude Code makes the change, and reads the result back.

The person then wants the gaps back, and says so. `show me` reopens the thread in a
terminal, where the diffs can be reviewed.
See [the tell command](cli.md#telling-your-agent-to-change-the-desktop).

### A focus mode, built and then removed

<https://github.com/user-attachments/assets/830cb5c8-2c52-4a2d-b7e9-e3d4bb252955>

The person taps `Super` + `F5` and asks for a focus mode. Claude Code dims the inactive
windows, turns the animations off, and binds `Super` + `Shift` + `F` to the top bar.

From 0:32 the new key toggles the top bar on screen, three times. A second request puts
the desktop back, and the key returns to the file manager.

## macOS

### A weather CLI made to work offline

<https://github.com/user-attachments/assets/b60f2c32-19cc-4da2-a0e2-463ecb9d56c8>

The person holds `Right Option` and asks Claude Code to make a weather CLI work offline.
Claude Code plans a cache, and the person asks for one JSON file kept inside the client.

After a code review, Claude Code reads out ten findings and says it would fix six. The
person asks for all ten, and asks what a read-only cache folder would show. Claude Code
ends by asking about the commit, and the person says to leave it uncommitted.

### A bug, found and fixed by voice

<https://github.com/user-attachments/assets/912c94af-baac-4385-b135-07a4eeb11b0e>

The person holds `Right Option` to talk. Claude Code finds a first-run bug in Banshee's
own code, says out loud what it would change, and asks how far to take the fix. The
answer is spoken back.
