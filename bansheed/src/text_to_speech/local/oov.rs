use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

// Optional espeak-ng subprocess that pronounces words misaki would otherwise
// spell out letter by letter.
pub struct OovFallback {
    bin: PathBuf,
    cache: Mutex<HashMap<String, Option<String>>>,
}

impl OovFallback {
    pub fn available() -> bool {
        resolve_espeak().is_some()
    }

    pub fn detect() -> Option<Self> {
        resolve_espeak().map(|bin| Self {
            bin,
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn phonemize(&self, word: &str) -> Option<String> {
        let key = word.to_lowercase();
        if let Some(hit) = self.cache.lock().unwrap().get(&key) {
            return hit.clone();
        }
        // Unlocked across the subprocess so lookups don't serialize on fork/exec.
        let result = run_espeak(&self.bin, &key);
        self.cache.lock().unwrap().insert(key, result.clone());
        result
    }
}

// Held for the life of the process once found: every utterance asks, and the
// search below costs a fork. A miss is not held, so an install lands at once.
static ESPEAK: OnceLock<PathBuf> = OnceLock::new();

pub(in crate::text_to_speech::local) fn resolve_espeak() -> Option<PathBuf> {
    if let Some(found) = ESPEAK.get() {
        return Some(found.clone());
    }
    let found = find_espeak()?;
    Some(ESPEAK.get_or_init(|| found).clone())
}

// launchd gives the daemon a minimal PATH, so fall back to the login shell,
// which knows about Homebrew/MacPorts/Nix prefixes.
fn find_espeak() -> Option<PathBuf> {
    if runs(Path::new("espeak-ng")) {
        return Some(PathBuf::from("espeak-ng"));
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let out = Command::new(shell)
        .args(["-lc", "command -v espeak-ng"])
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty() && runs(Path::new(&path))).then_some(PathBuf::from(path))
}

fn runs(bin: &Path) -> bool {
    Command::new(bin)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

pub(crate) fn espeak_install_hint() -> String {
    if cfg!(target_os = "macos") {
        return "brew install espeak-ng".to_string();
    }
    for (mgr, verb) in [
        ("apt", "install"),
        ("dnf", "install"),
        ("pacman", "-S"),
        ("zypper", "install"),
        ("apk", "add"),
    ] {
        if runs(Path::new(mgr)) {
            return format!("sudo {mgr} {verb} espeak-ng");
        }
    }
    "your package manager's espeak-ng package".to_string()
}

fn run_espeak(bin: &Path, word: &str) -> Option<String> {
    let output = Command::new(bin)
        .args(["-q", "--ipa", "-v", "en-us"])
        .arg(word)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let ipa = String::from_utf8_lossy(&output.stdout);
    let cleaned = clean(&ipa);
    (!cleaned.is_empty()).then_some(cleaned)
}

// Strip joiners/pauses espeak may emit and collapse doubled stress marks.
fn clean(ipa: &str) -> String {
    ipa.chars()
        .filter(|c| !matches!(c, '\u{200d}' | '\u{0361}' | '_'))
        .collect::<String>()
        .replace("ˈˈ", "ˈ")
        .replace("ˌˌ", "ˌ")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn clean_strips_joiners_and_trims() {
        // ZWJ and tie bar drop; surrounding whitespace and doubled stress collapse
        assert_eq!(clean(" d\u{200d}ʒˈˈʌd\u{361}ʒ \n"), "dʒˈʌdʒ");
    }
}
