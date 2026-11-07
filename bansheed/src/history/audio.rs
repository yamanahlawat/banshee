//! Kept audio: one `<id>.wav` per History row, in a folder the caller names.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use banshee_common::utils::{make_owner_only, write_atomically};
use serde::Serialize;

use crate::speech_to_text::SAMPLE_RATE;
use crate::speech_to_text::remote::wav::{WAV_HEADER_BYTES, pcm16_wav, pcm16_wav_samples};

/// 16-bit mono.
const BYTES_PER_SECOND: u64 = SAMPLE_RATE as u64 * 2;

#[derive(Serialize, Debug, PartialEq)]
pub struct Kept {
    pub seconds: f64,
    pub until: Option<String>,
}

pub fn path(dir: &Path, id: i64) -> PathBuf {
    dir.join(format!("{id}.wav"))
}

pub fn prepare(dir: &Path) -> std::io::Result<()> {
    make_owner_only(dir)?;
    exclude_from_backup(dir);
    Ok(())
}

#[cfg(target_os = "macos")]
fn exclude_from_backup(dir: &Path) {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSNumber, NSString, NSURL, NSURLIsExcludedFromBackupKey};

    let url = NSURL::fileURLWithPath(&NSString::from_str(&dir.to_string_lossy()));
    let yes = NSNumber::numberWithBool(true);
    // SAFETY: the backup-exclusion key takes an NSNumber boolean.
    let result = unsafe {
        url.setResourceValue_forKey_error(Some(&*yes as &AnyObject), NSURLIsExcludedFromBackupKey)
    };
    if let Err(error) = result {
        log::error!(
            "could not exclude {} from backups: {error:?}",
            dir.display()
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn exclude_from_backup(_dir: &Path) {}

pub fn write(dir: &Path, id: i64, samples: &[f32]) -> std::io::Result<()> {
    if !dir.is_dir() {
        prepare(dir)?;
    }
    write_atomically(
        &path(dir, id),
        &pcm16_wav(samples, SAMPLE_RATE),
        Some(0o600),
    )
}

pub fn read(file: &Path) -> std::io::Result<Vec<f32>> {
    Ok(pcm16_wav_samples(&std::fs::read(file)?))
}

pub fn kept(dir: &Path, id: i64, period: Option<Duration>) -> Option<Kept> {
    described(path(dir, id), period)
}

/// The kept audio of each of `ids` that has a file, in one pass over the folder.
pub fn kept_of(dir: &Path, ids: &HashSet<i64>, period: Option<Duration>) -> HashMap<i64, Kept> {
    files(dir)
        .filter(|(id, _)| ids.contains(id))
        .filter_map(|(id, path)| Some((id, described(path, period)?)))
        .collect()
}

fn described(path: PathBuf, period: Option<Duration>) -> Option<Kept> {
    let meta = std::fs::metadata(path).ok()?;
    let seconds =
        meta.len().saturating_sub(WAV_HEADER_BYTES as u64) as f64 / BYTES_PER_SECOND as f64;
    let until =
        period.and_then(|period| Some(super::stamp((meta.modified().ok()? + period).into())));
    Some(Kept { seconds, until })
}

/// The folder's `<integer>.wav` files, by row id.
fn files(dir: &Path) -> impl Iterator<Item = (i64, PathBuf)> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter_map(|path| {
            if path.extension().is_none_or(|ext| ext != "wav") {
                return None;
            }
            let id = path.file_stem()?.to_str()?.parse::<i64>().ok()?;
            Some((id, path))
        })
}

pub fn purge(dir: &Path, period: Duration, now: SystemTime) -> usize {
    let mut removed = 0;
    for (_, path) in files(dir) {
        let modified = match std::fs::metadata(&path).and_then(|meta| meta.modified()) {
            Ok(modified) => modified,
            Err(error) => {
                warn_unless_gone(&path, &error);
                continue;
            }
        };
        if modified + period > now {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(error) => warn_unless_gone(&path, &error),
        }
    }
    removed
}

/// A file already gone is a concurrent delete, not a fault.
fn warn_unless_gone(path: &Path, error: &std::io::Error) {
    if error.kind() != std::io::ErrorKind::NotFound {
        log::warn!("kept audio {}: {error}", path.display());
    }
}

pub fn remove_all(dir: &Path) -> std::io::Result<()> {
    for (_, path) in files(dir) {
        match std::fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{DAY, scratch, set_modified};
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn prepare_makes_the_folder_owner_only() {
        let dir = scratch("audio-prepare").join("audio");
        prepare(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn prepare_tightens_a_folder_that_already_exists() {
        let dir = scratch("audio-prepare-existing");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        prepare(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn a_written_file_is_owner_only_and_reads_back() {
        let dir = scratch("audio-write");
        let samples = vec![0.25_f32; 8_000];
        write(&dir, 3, &samples).unwrap();
        assert_eq!(mode(&path(&dir, 3)), 0o600);
        let back = read(&path(&dir, 3)).unwrap();
        assert_eq!(back.len(), samples.len());
        assert!((back[0] - 0.25).abs() < 1e-3);
        assert_eq!(kept(&dir, 3, None).unwrap().seconds, 0.5);
    }

    #[test]
    fn a_write_into_a_removed_folder_makes_it_owner_only_again() {
        let dir = scratch("audio-write-removed").join("audio");
        prepare(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        write(&dir, 1, &[0.0; 16]).unwrap();
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn kept_is_none_without_a_file() {
        let dir = scratch("audio-absent");
        assert!(kept(&dir, 1, Some(DAY)).is_none());
    }

    #[test]
    fn kept_until_is_none_when_kept_forever() {
        let dir = scratch("audio-forever");
        write(&dir, 1, &[0.0; 16]).unwrap();
        assert_eq!(kept(&dir, 1, None).unwrap().until, None);
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        set_modified(&path(&dir, 1), modified);
        let until = kept(&dir, 1, Some(DAY)).unwrap().until.unwrap();
        let named = chrono::DateTime::parse_from_rfc3339(&until).unwrap();
        assert_eq!(SystemTime::from(named), modified + DAY);
    }

    #[test]
    fn purge_removes_only_files_past_the_period() {
        let dir = scratch("audio-purge");
        let now = SystemTime::now();
        write(&dir, 1, &[0.0; 16]).unwrap();
        write(&dir, 2, &[0.0; 16]).unwrap();
        set_modified(&path(&dir, 1), now - 8 * DAY);
        set_modified(&path(&dir, 2), now - 6 * DAY);
        assert_eq!(purge(&dir, 7 * DAY, now), 1);
        assert!(!path(&dir, 1).exists());
        assert!(path(&dir, 2).exists());
    }

    #[test]
    fn purge_with_a_zero_period_removes_everything() {
        let dir = scratch("audio-purge-zero");
        write(&dir, 1, &[0.0; 16]).unwrap();
        write(&dir, 2, &[0.0; 16]).unwrap();
        assert_eq!(purge(&dir, Duration::ZERO, SystemTime::now()), 2);
    }

    #[test]
    fn purge_and_remove_all_leave_other_names() {
        let dir = scratch("audio-others");
        std::fs::write(dir.join("notes.txt"), "keep").unwrap();
        write(&dir, 1, &[0.0; 16]).unwrap();
        purge(&dir, Duration::ZERO, SystemTime::now());
        write(&dir, 2, &[0.0; 16]).unwrap();
        remove_all(&dir).unwrap();
        assert!(dir.join("notes.txt").exists());
        assert!(!path(&dir, 2).exists());
    }
}
