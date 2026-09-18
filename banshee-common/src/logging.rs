//! The log sink every Banshee process installs. One line per record with a
//! clock, so an interval in a log can be measured. `BANSHEE_LOG` picks the
//! level, `info` when it is unset, so a dictated sentence reaches a log only
//! when asked for.

use std::io::Write;

use log::{LevelFilter, Log, Metadata, Record};

struct Sink;

impl Log for Sink {
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let clock = chrono::Local::now().format("%H:%M:%S%.3f");
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "{}", line(clock, record));
    }

    fn flush(&self) {}
}

/// Installs the sink at the level `BANSHEE_LOG` names. A second call is a
/// no-op, so a test that starts two daemons installs one sink.
pub fn install() {
    let level = level_from(std::env::var("BANSHEE_LOG").ok().as_deref());
    if log::set_logger(&Sink).is_ok() {
        log::set_max_level(level);
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
