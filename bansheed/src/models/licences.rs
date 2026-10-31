use std::path::PathBuf;

use banshee_common::{error::BansheeError, listening::ListeningModel};

/// What the person has accepted. The CLI, the daemon and the window share one
/// record on disk, read once when this is built.
pub struct Accepted {
    path: Option<PathBuf>,
    licences: Vec<String>,
}

impl Accepted {
    #[cfg(test)]
    pub fn at(path: PathBuf) -> Self {
        Self::read(Some(path))
    }

    pub fn in_banshee_dir() -> Self {
        Self::read(banshee_common::utils::accepted_licences_path())
    }

    // A record that cannot be read accepts nothing.
    fn read(path: Option<PathBuf>) -> Self {
        let licences = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| text.lines().map(str::to_string).collect())
            .unwrap_or_default();
        Self { path, licences }
    }

    pub fn holds(&self, licence: &str) -> bool {
        self.licences.iter().any(|held| held == licence)
    }

    /// Whether `model`'s licence asks for an acceptance this record lacks.
    pub fn blocks(&self, model: &ListeningModel) -> bool {
        model.licence.needs_acceptance && !self.holds(model.licence.name)
    }

    pub fn add(&mut self, licence: &str) -> Result<(), BansheeError> {
        let path = self.path.as_ref().ok_or_else(|| {
            BansheeError::Other(
                "Could not find the home directory to record the licence".to_string(),
            )
        })?;
        if self.holds(licence) {
            return Ok(());
        }
        let mut text = std::fs::read_to_string(path).unwrap_or_default();
        text.push_str(licence);
        text.push('\n');
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        banshee_common::utils::write_atomically(path, text.as_bytes(), None)?;
        self.licences.push(licence.to_string());
        Ok(())
    }
}

/// Why `model` cannot download yet: its licence, what accepting it means,
/// where to read it, and the flag that accepts it. `None` when nothing stands
/// in the way.
pub fn refusal(model: &ListeningModel, accepted: &Accepted) -> Option<String> {
    let licence = model.licence;
    accepted.blocks(model).then(|| {
        let credit = licence
            .credit
            .map(|credit| format!(" Credit: {credit}."))
            .unwrap_or_default();
        format!(
            "{} is under the {}.{credit} {} Read it at {}. To accept it and download: {}",
            model.id,
            licence.name,
            licence.summary,
            licence.url,
            accept_command(model)
        )
    })
}

pub fn accept_command(model: &ListeningModel) -> String {
    format!("banshee setup --model {} --accept-licence", model.id)
}

#[cfg(test)]
mod tests {
    use super::{Accepted, refusal};
    use banshee_common::listening::{DEFAULT_MODEL, find};

    #[test]
    fn an_accepted_licence_is_remembered_and_no_other() {
        let dir = crate::test_support::unique_scratch("licences");
        let mut accepted = Accepted::at(dir.join("accepted-licences"));
        let canary = find("canary-1b").unwrap();
        let granite = find("granite-speech-5.0-470m-turboctc-nc").unwrap();

        assert!(refusal(canary, &accepted).is_some());
        accepted.add(canary.licence.name).unwrap();
        accepted.add(canary.licence.name).unwrap();
        let record = std::fs::read_to_string(dir.join("accepted-licences")).unwrap();
        let canary_after = refusal(canary, &accepted);
        let granite_after = refusal(granite, &accepted);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(record, "CC-BY-NC-4.0\n");
        assert_eq!(canary_after, None);
        assert!(granite_after.unwrap().contains("CC-BY-NC-SA-4.0"));
    }

    #[test]
    fn a_moonshine_refusal_carries_its_credit_and_terms() {
        let accepted = Accepted::at("/no-such-dir-9f3a/accepted-licences".into());
        let reason = refusal(find("moonshine-base-zh").unwrap(), &accepted).unwrap();
        for part in [
            "Moonshine AI Community License (2025-06-15)",
            "Credit: Powered by Moonshine AI.",
            "more than USD 1M annual revenue",
            "https://github.com/moonshine-ai/moonshine/blob/main/LICENSE",
            "banshee setup --model moonshine-base-zh --accept-licence",
        ] {
            assert!(reason.contains(part), "{reason}");
        }
    }

    #[test]
    fn a_licence_that_allows_commercial_use_asks_nothing() {
        let accepted = Accepted::at("/no-such-dir-9f3a/accepted-licences".into());
        assert_eq!(refusal(find(DEFAULT_MODEL).unwrap(), &accepted), None);
    }
}
