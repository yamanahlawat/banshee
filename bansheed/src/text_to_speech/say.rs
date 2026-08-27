use std::process::{Child, Command};

use super::{ActiveUtterance, TtsBackend};

// macOS `say` fallback: zero-download, one child process per utterance
pub struct SayBackend;

impl TtsBackend for SayBackend {
    fn start(&self, text: &str, _voice: Option<&str>) -> std::io::Result<Box<dyn ActiveUtterance>> {
        let child = Command::new("say").arg(text).spawn()?;
        Ok(Box::new(SayUtterance { child }))
    }
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
