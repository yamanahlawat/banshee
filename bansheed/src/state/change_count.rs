use tokio::sync::watch;

/// A counter a reader waits on. The value says that something moved, not what:
/// the reader reads the state itself.
pub struct ChangeCount(watch::Sender<u64>);

impl ChangeCount {
    pub fn new() -> Self {
        Self(watch::channel(0).0)
    }

    pub fn bump(&self) {
        self.0.send_modify(|count| *count += 1);
    }

    pub fn now(&self) -> u64 {
        *self.0.borrow()
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.0.subscribe()
    }
}
