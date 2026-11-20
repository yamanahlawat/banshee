//! The log sink every Banshee process installs. One line per record with a
//! clock, so an interval in a log can be measured. `BANSHEE_LOG` picks the
//! level, `info` when it is unset, so a dictated sentence reaches a log only
//! when asked for. On macOS a program launchd starts carries
//! `BANSHEE_LOG_TO=file` and writes to its own file under `~/Library/Logs`,
//! since launchd discards stderr. Every other run writes to stderr.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use log::{LevelFilter, Log, Metadata, Record};

enum Sink {
    Stderr,
    File(Mutex<Rotating>),
}

// macOS rotates its own logs in /etc/newsyslog.conf at 1000 KB, keeping five
// old files.
const ROTATE_AT_BYTES: u64 = 1000 * 1024;
const OLD_FILES_KEPT: usize = 5;

impl Sink {
    fn file(path: &Path) -> std::io::Result<Self> {
        Ok(Sink::File(Mutex::new(Rotating::new(
            path,
            ROTATE_AT_BYTES,
            OLD_FILES_KEPT,
        )?)))
    }
}

/// An append-only log that moves itself to `<path>.1` once it has grown past
/// `limit` bytes, shifting older files up to `<path>.<kept>`.
struct Rotating {
    path: PathBuf,
    limit: u64,
    kept: usize,
    file: File,
    written: u64,
}

impl Rotating {
    fn new(path: &Path, limit: u64, kept: usize) -> std::io::Result<Self> {
        let (file, written) = open(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            limit,
            kept,
            file,
            written,
        })
    }

    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if !self.path.exists() {
            (self.file, self.written) = open(&self.path)?;
        }
        if self.written > self.limit {
            self.rotate()?;
        }
        self.file.write_all(bytes)?;
        self.written += bytes.len() as u64;
        Ok(())
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        for n in (1..self.kept).rev() {
            moved(&self.old(n), &self.old(n + 1))?;
        }
        moved(&self.path, &self.old(1))?;
        (self.file, self.written) = open(&self.path)?;
        Ok(())
    }

    fn old(&self, n: usize) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(format!(".{n}"));
        name.into()
    }
}

fn open(path: &Path) -> std::io::Result<(File, u64)> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let file = File::options().create(true).append(true).open(path)?;
    let written = file.metadata()?.len();
    Ok((file, written))
}

fn moved(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn clock(at: &jiff::Zoned) -> impl std::fmt::Display {
    at.strftime("%H:%M:%S%.3f")
}

/// transcribe.cpp writes many Info lines on every model load, and
/// the daemon logs the backend it settled on itself.
const QUIET_BELOW_WARN: &str = "transcribe_cpp";

fn wanted(metadata: &Metadata) -> bool {
    metadata.target() != QUIET_BELOW_WARN || metadata.level() <= log::Level::Warn
}

impl Log for Sink {
    fn enabled(&self, metadata: &Metadata) -> bool {
        wanted(metadata)
    }

    fn log(&self, record: &Record) {
        if !wanted(record.metadata()) {
            return;
        }
        let line = format!("{}\n", line(clock(&jiff::Zoned::now()), record));
        match self {
            Sink::Stderr => {
                let _ = std::io::stderr().lock().write_all(line.as_bytes());
            }
            Sink::File(file) => {
                if let Ok(mut file) = file.lock() {
                    let _ = file.write(line.as_bytes());
                }
            }
        }
    }

    fn flush(&self) {}
}

static SINK: OnceLock<Sink> = OnceLock::new();

/// Installs the sink at the level `BANSHEE_LOG` names. `program` names the log
/// file, `daemon` or `tray`. A second call is a no-op, so a test that starts
/// two daemons installs one sink.
pub fn install(program: &str) {
    let level = level_from(std::env::var("BANSHEE_LOG").ok().as_deref());
    let sink = SINK.get_or_init(|| {
        let to_file = cfg!(target_os = "macos")
            && std::env::var("BANSHEE_LOG_TO").is_ok_and(|to| to == "file");
        crate::utils::log_path(program)
            .filter(|_| to_file)
            .and_then(|path| Sink::file(&path).ok())
            .unwrap_or(Sink::Stderr)
    });
    if log::set_logger(sink).is_ok() {
        log::set_max_level(level);
        if matches!(sink, Sink::File(_)) {
            let report = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |panic| {
                log::error!("{panic}");
                report(panic);
            }));
        }
    }
}

/// Says why the process ends: in the log file when launchd started it, and on
/// stderr otherwise.
pub fn failure(line: &str) {
    match SINK.get() {
        Some(Sink::File(_)) => log::error!("{line}"),
        _ => eprintln!("{line}"),
    }
}

fn level_from(setting: Option<&str>) -> LevelFilter {
    setting
        .and_then(|word| word.parse().ok())
        .unwrap_or(LevelFilter::Info)
}

fn line(clock: impl std::fmt::Display, record: &Record) -> String {
    // The crate's own module, not the crate: every line is Banshee's
    let module = record
        .target()
        .rsplit("::")
        .next()
        .unwrap_or(record.target());
    format!("{clock} {:<5} {module}: {}", record.level(), record.args())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logged_line_lands_in_the_file() {
        let dir = std::env::temp_dir().join(format!("banshee-log-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("Logs/daemon.log");
        std::fs::create_dir_all(dir.join("Logs")).unwrap();
        std::fs::write(&path, "a line from an earlier run\n").unwrap();
        let sink = Sink::file(&path).expect("a sink on a temp folder");
        let unique = format!("a line only this test writes {}", std::process::id());
        for _ in 0..2 {
            sink.log(
                &log::Record::builder()
                    .level(log::Level::Info)
                    .target("banshee::daemon")
                    .args(format_args!("{unique}"))
                    .build(),
            );
        }

        let written = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            written.starts_with("a line from an earlier run\n"),
            "{written:?}"
        );
        assert_eq!(written.matches(&unique).count(), 2, "{written:?}");
        assert!(
            written.ends_with(&format!("INFO  daemon: {unique}\n")),
            "{written:?}"
        );
    }

    fn scratch_log(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("banshee-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("daemon.log");
        (dir, path)
    }

    fn read(path: &std::path::Path, suffix: &str) -> String {
        std::fs::read_to_string(format!("{}{suffix}", path.display())).unwrap_or_default()
    }

    #[test]
    fn a_file_past_the_limit_moves_aside_and_the_next_line_starts_a_new_one() {
        let (dir, path) = scratch_log("rotate");
        let mut file = Rotating::new(&path, 10, 5).unwrap();
        file.write(b"0123456789A\n").unwrap();
        file.write(b"next\n").unwrap();

        let (current, old) = (read(&path, ""), read(&path, ".1"));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(old, "0123456789A\n");
        assert_eq!(current, "next\n");
    }

    #[test]
    fn only_the_newest_old_files_are_kept() {
        let (dir, path) = scratch_log("rotate-kept");
        let mut file = Rotating::new(&path, 1, 2).unwrap();
        for line in ["a\n", "b\n", "c\n", "d\n"] {
            file.write(line.as_bytes()).unwrap();
        }

        let found = [read(&path, ""), read(&path, ".1"), read(&path, ".2")];
        let third_kept = std::fs::exists(format!("{}.3", path.display())).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found, ["d\n", "c\n", "b\n"]);
        assert!(!third_kept, "a third old file outlived the limit of two");
    }

    #[test]
    fn a_log_deleted_while_running_comes_back_on_the_next_line() {
        let (dir, path) = scratch_log("rotate-deleted");
        let mut file = Rotating::new(&path, 1024, 5).unwrap();
        file.write(b"before\n").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        file.write(b"after\n").unwrap();

        let current = read(&path, "");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(current, "after\n");
    }

    #[test]
    fn the_clock_reads_hours_to_milliseconds() {
        let at: jiff::Zoned = "2026-11-17T21:33:17.25+05:30[+05:30]".parse().unwrap();
        assert_eq!(clock(&at).to_string(), "21:33:17.250");
    }

    #[test]
    fn transcribe_cpp_reaches_the_log_only_from_warn_up() {
        let at =
            |target, level| wanted(&log::Metadata::builder().target(target).level(level).build());
        assert!(!at("transcribe_cpp", log::Level::Info));
        assert!(at("transcribe_cpp", log::Level::Warn));
        assert!(at("transcribe_cpp", log::Level::Error));
        assert!(at("banshee::daemon", log::Level::Info));
    }

    #[test]
    fn an_unset_level_reads_info() {
        assert_eq!(level_from(None), log::LevelFilter::Info);
    }

    #[test]
    fn a_level_is_read_whatever_its_case() {
        assert_eq!(level_from(Some("DEBUG")), log::LevelFilter::Debug);
        assert_eq!(level_from(Some("trace")), log::LevelFilter::Trace);
    }

    #[test]
    fn a_word_that_is_no_level_falls_back_to_info() {
        assert_eq!(level_from(Some("loud")), log::LevelFilter::Info);
    }

    #[test]
    fn a_line_carries_the_clock_the_level_and_the_module() {
        let record = log::Record::builder()
            .level(log::Level::Warn)
            .target("banshee::hotkey")
            .args(format_args!("push-to-talk ran past 90s"))
            .build();
        assert_eq!(
            line("12:00:01.500", &record),
            "12:00:01.500 WARN  hotkey: push-to-talk ran past 90s"
        );
    }
}
