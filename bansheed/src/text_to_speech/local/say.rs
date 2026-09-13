use std::io;
use std::process::{Child, Command};

#[cfg(not(target_os = "macos"))]
use crate::text_to_speech::local::oov::{espeak_install_hint, resolve_espeak};
use crate::text_to_speech::{ActiveUtterance, TtsBackend};

// System-voice fallback: zero-download, one child process per utterance.
// macOS runs `say`; every other target runs `espeak-ng`.
pub struct SayBackend;

/// The error a backend with one fixed voice returns for a per-request voice.
pub(crate) const VOICE_NEEDS_KOKORO: &str =
    "choosing a voice needs the Kokoro backend, which is not loaded";

impl TtsBackend for SayBackend {
    fn start(&self, text: &str, voice: Option<&str>) -> io::Result<Box<dyn ActiveUtterance>> {
        if voice.is_some() {
            return Err(io::Error::other(VOICE_NEEDS_KOKORO));
        }
        let child = command_for(text)?.spawn()?;
        Ok(Box::new(SayUtterance { child }))
    }
}

#[cfg(target_os = "macos")]
fn program() -> &'static str {
    "say"
}

#[cfg(not(target_os = "macos"))]
fn program() -> &'static str {
    "espeak-ng"
}

#[cfg(target_os = "macos")]
fn command_for(text: &str) -> io::Result<Command> {
    let mut command = Command::new(program());
    command.arg(text);
    Ok(command)
}

#[cfg(not(target_os = "macos"))]
fn command_for(text: &str) -> io::Result<Command> {
    let bin = resolve_espeak().ok_or_else(|| {
        io::Error::other(format!(
            "{} not installed; install: {}",
            program(),
            espeak_install_hint()
        ))
    })?;
    let mut command = Command::new(bin);
    command.arg(text);
    Ok(command)
}

struct SayUtterance {
    child: Child,
}

impl ActiveUtterance for SayUtterance {
    fn is_finished(&mut self) -> bool {
        // try_wait returns Some(status) once the process has exited
        !matches!(self.child.try_wait(), Ok(None))
    }

    fn stop(&mut self) {
        // kill only signals; wait reaps the zombie process entry
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_this_backend_cannot_apply_is_refused() {
        let Err(error) = SayBackend.start("hello", Some("am_adam")) else {
            panic!("say cannot apply a Kokoro voice, so it must refuse");
        };
        assert!(error.to_string().contains("Kokoro"), "{error}");
    }

    #[test]
    fn the_fallback_names_the_platform_voice() {
        #[cfg(target_os = "macos")]
        let expected = "say";
        #[cfg(not(target_os = "macos"))]
        let expected = "espeak-ng";
        assert_eq!(program(), expected);
    }
}
