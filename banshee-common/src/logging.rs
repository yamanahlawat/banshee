//! The log sink every Banshee process installs. One line per record with a
//! clock, so an interval in a log can be measured. `BANSHEE_LOG` picks the
//! level, `info` when it is unset, so a dictated sentence reaches a log only
//! when asked for. On macOS a program launchd starts carries
//! `BANSHEE_LOG_TO=file` and writes to its own file under `~/Library/Logs`,
//! since launchd discards stderr. Every other run writes to stderr.

use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use file_rotate::compression::Compression;
use file_rotate::suffix::AppendCount;
use file_rotate::{ContentLimit, FileRotate};
use log::{LevelFilter, Log, Metadata, Record};

enum Sink {
    Stderr,
    File(Mutex<FileRotate<AppendCount>>),
}

// macOS rotates its own logs in /etc/newsyslog.conf at 1000 KB, keeping five
// old files; file-rotate has no default of its own.
const ROTATE_AT_BYTES: usize = 1000 * 1024;
const OLD_FILES_KEPT: usize = 5;

impl Sink {
    fn file(path: &Path) -> std::io::Result<Self> {
        // FileRotate panics when it cannot make the folder, so the folder is made here first.
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        Ok(Sink::File(Mutex::new(FileRotate::new(
            path,
            AppendCount::new(OLD_FILES_KEPT),
            ContentLimit::BytesSurpassed(ROTATE_AT_BYTES),
            Compression::None,
            None,
        ))))
    }
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
        let clock = chrono::Local::now().format("%H:%M:%S%.3f");
        let line = format!("{}\n", line(clock, record));
        match self {
            Sink::Stderr => {
                let _ = std::io::stderr().lock().write_all(line.as_bytes());
            }
            Sink::File(file) => {
                if let Ok(mut file) = file.lock() {
                    let _ = file.write_all(line.as_bytes());
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
