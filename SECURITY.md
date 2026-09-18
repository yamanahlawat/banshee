# Security

Banshee holds a microphone, an accessibility grant that lets it type into any
window, and the API keys in `~/.banshee/credentials.toml`. A fault in any of
those is a security fault.

## Report a fault

- Use GitHub's private vulnerability reporting on this repository: the
  **Security** tab, then **Report a vulnerability**.
- Do not open a public issue for it.
- Say which version `banshee status` reports, what you did, and what you saw.

## What to expect

- An acknowledgement within seven days.
- A fix in the next release, or a reason why not, within thirty days of a
  confirmed report.
- Credit in the changelog, unless you ask for none.

## What Banshee does with what it holds

- Audio never leaves the machine unless `[stt]` or `[tts]` names a remote
  provider, and then only to the `base_url` you set.
- The key file is written owner-only, and `banshee status` reports one that
  others can read.
- The daemon's socket is owner-only. Anything that can reach it can hear the
  microphone and speak through the speakers.
