// Start-at-login behind a neutral surface: launchd on macOS, systemd on Linux.

use std::path::PathBuf;

use banshee_common::error::BansheeError;

/// The platform arms decide what they honour, so a new entry is a variant, not a new pair of
/// functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Daemon,
    Tray,
}

impl Agent {
    pub const ALL: [Agent; 2] = [Agent::Daemon, Agent::Tray];

    pub fn name(self) -> &'static str {
        match self {
            Agent::Daemon => "daemon",
            Agent::Tray => "tray",
        }
    }
}

/// The log level the installer ran with. A supervisor hands its agent an
/// environment of its own, so `BANSHEE_LOG` reaches a supervised daemon only
/// when the service file carries it.
#[cfg(target_os = "linux")]
fn supervised_level() -> Option<String> {
    std::env::var("BANSHEE_LOG")
        .ok()
        .filter(|level| !level.trim().is_empty())
}

pub(crate) fn home_dir() -> Result<PathBuf, BansheeError> {
    dirs::home_dir().ok_or_else(|| BansheeError::Other("home dir not found".into()))
}

#[cfg(target_os = "macos")]
mod login_items {
    pub(super) const LAUNCHCTL: &str = "/bin/launchctl";

    use std::path::{Path, PathBuf};
    use std::process::Command;

    use banshee_common::error::BansheeError;
    use banshee_common::utils::{
        DAEMON_AGENT, LEGACY_AGENTS, Registration, TRAY_AGENT, launchd_target, log_path, sibling,
    };

    use super::{Agent, home_dir};

    fn label(agent: Agent) -> &'static str {
        match agent {
            Agent::Daemon => DAEMON_AGENT,
            Agent::Tray => TRAY_AGENT,
        }
    }

    /// Boots out each legacy job whose plist is under `home`, then deletes the
    /// plist, and returns the plists it deleted. A plist that stays is an error
    /// naming it, since launchd loads it again at the next login.
    pub fn migrate_legacy(
        home: &Path,
        bootout: impl Fn(&str),
    ) -> Result<Vec<PathBuf>, BansheeError> {
        let mut removed = Vec::new();
        for label in LEGACY_AGENTS {
            let plist = home
                .join("Library/LaunchAgents")
                .join(format!("{label}.plist"));
            if !plist.exists() {
                continue;
            }
            bootout(label);
            std::fs::remove_file(&plist).map_err(|error| {
                BansheeError::Other(format!("could not delete {}: {error}", plist.display()))
            })?;
            removed.push(plist);
        }
        Ok(removed)
    }

    /// Finds `banshee-app` with `find_app`, and only then migrates the legacy
    /// jobs, so a binary with no app beside it leaves the old jobs running.
    pub(super) fn ready_to_register(
        home: &Path,
        find_app: impl FnOnce() -> Result<PathBuf, BansheeError>,
        bootout: impl Fn(&str),
    ) -> Result<PathBuf, BansheeError> {
        let app = find_app()?;
        migrate_legacy(home, bootout)?;
        Ok(app)
    }

    /// The `banshee-app` inside the same bundle as this binary.
    fn the_app() -> Result<PathBuf, BansheeError> {
        sibling(&std::env::current_exe()?, "banshee-app")
    }

    fn bootout_legacy(label: &str) {
        // A job that is not loaded fails the bootout, and its plist still goes.
        let _ = Command::new(LAUNCHCTL)
            .args(["bootout", &launchd_target(label)])
            .output();
    }

    /// The state `banshee-app --login-items` printed for `label`.
    pub(super) fn registration_in(stdout: &str, label: &str) -> Option<Registration> {
        stdout.lines().find_map(|line| {
            let (named, word) = line.split_once(' ')?;
            if named == label {
                Registration::from_word(word.trim())
            } else {
                None
            }
        })
    }

    pub fn awaits_approval(agent: Agent) -> String {
        let command = match agent {
            Agent::Daemon => "banshee start",
            Agent::Tray => "banshee tray",
        };
        format!(
            "macOS has Banshee's {} switched off. Switch Banshee on in System Settings > \
             General > Login Items, then run {command} again.",
            agent.name()
        )
    }

    /// What one run of `banshee-app --login-items` says about `label`.
    pub(super) fn outcome(
        success: bool,
        stdout: &str,
        stderr: &str,
        label: &str,
    ) -> Result<Registration, BansheeError> {
        match registration_in(stdout, label) {
            Some(state) if success => Ok(state),
            _ if stderr.trim().is_empty() => Err(BansheeError::Other(format!(
                "banshee-app printed no state for {label}"
            ))),
            _ => Err(BansheeError::Other(format!(
                "banshee-app could not settle {label}: {}",
                stderr.trim()
            ))),
        }
    }

    fn login_items(app: &Path, action: &str, agent: Agent) -> Result<Registration, BansheeError> {
        let output = Command::new(app)
            .args(["--login-items", action, agent.name()])
            .output()?;
        outcome(
            output.status.success(),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
            label(agent),
        )
    }

    pub fn registration(agent: Agent) -> Result<Registration, BansheeError> {
        login_items(&the_app()?, "status", agent)
    }

    pub fn follow_the_log(agent: Agent) -> Result<String, BansheeError> {
        let log = log_path(agent.name())
            .ok_or_else(|| BansheeError::Other("home dir not found".into()))?;
        Ok(format!("tail -f {}", log.display()))
    }

    /// Returns the command that follows the agent's log.
    fn register(app: &Path, agent: Agent) -> Result<String, BansheeError> {
        match login_items(app, "register", agent)? {
            Registration::Enabled => follow_the_log(agent),
            Registration::RequiresApproval => Err(BansheeError::Other(awaits_approval(agent))),
            other => Err(BansheeError::Other(format!(
                "macOS reports the {} {} after registering it",
                agent.name(),
                other.word()
            ))),
        }
    }

    /// True when the agent was registered or a legacy plist was removed. A
    /// binary with no `banshee-app` beside it is outside a bundle, so it has no
    /// registration to remove.
    pub fn uninstall(agent: Agent) -> Result<bool, BansheeError> {
        let migrated = !migrate_legacy(&home_dir()?, bootout_legacy)?.is_empty();
        let Ok(app) = the_app() else {
            return Ok(migrated);
        };
        if !login_items(&app, "status", agent)?.is_registered() {
            return Ok(migrated);
        }
        unregister(&app, agent)?;
        Ok(true)
    }

    fn unregister(app: &Path, agent: Agent) -> Result<(), BansheeError> {
        match login_items(app, "unregister", agent)? {
            now if !now.is_registered() => Ok(()),
            other => Err(BansheeError::Other(format!(
                "macOS reports the {} {} after unregistering it",
                agent.name(),
                other.word()
            ))),
        }
    }

    /// Registers the agent, unregistering it first when macOS has it enabled.
    /// `SMAppService` has no call that starts a stopped agent, and Apple's
    /// header asks for both calls after the executable changes.
    pub fn start(agent: Agent) -> Result<String, BansheeError> {
        let app = ready_to_register(&home_dir()?, the_app, bootout_legacy)?;
        if login_items(&app, "status", agent)? == Registration::Enabled {
            unregister(&app, agent)?;
        }
        register(&app, agent)
    }
}

#[cfg(target_os = "macos")]
pub use login_items::{awaits_approval, follow_the_log, registration, start, uninstall};

#[cfg(target_os = "linux")]
mod systemd {
    pub(super) const SYSTEMCTL: &str = "/usr/bin/systemctl";

    use std::path::PathBuf;

    use super::Agent;
    use std::process::Command;

    use banshee_common::error::BansheeError;
    use banshee_common::utils::{DAEMON_AGENT, TRAY_AGENT, sibling, systemd_unit};

    fn label(agent: Agent) -> &'static str {
        match agent {
            Agent::Daemon => DAEMON_AGENT,
            Agent::Tray => TRAY_AGENT,
        }
    }

    fn unit_name(agent: Agent) -> &'static str {
        systemd_unit(label(agent)).expect("every agent names its own unit")
    }

    fn unit_path(agent: Agent) -> Option<PathBuf> {
        Some(
            dirs::config_dir()?
                .join("systemd/user")
                .join(unit_name(agent)),
        )
    }

    pub fn service_file_path() -> Option<PathBuf> {
        unit_path(Agent::Daemon)
    }

    /// The command this binary runs as, quoted the way a shell needs.
    fn exec_start(agent: Agent) -> Result<String, BansheeError> {
        let binary = std::env::current_exe()?;
        Ok(match agent {
            Agent::Daemon => format!("\"{}\" serve", binary.display()),
            Agent::Tray => format!("\"{}\"", sibling(&binary, "banshee-tray")?.display()),
        })
    }

    /// The daemon opens the microphone and is useful with no desktop, so it
    /// waits on pipewire and wants the boot target. The tray has no bar to
    /// sit in without a session, so it waits on the session instead, and
    /// stops when the session does.
    pub(super) fn content(agent: Agent, exec_start: &str, level: Option<&str>) -> String {
        let environment = level.map_or(String::new(), |level| {
            format!("Environment=BANSHEE_LOG={level}\n")
        });
        match agent {
            Agent::Daemon => format!(
                r#"[Unit]
Description=Banshee voice daemon
After=pipewire.service

[Service]
ExecStart={exec_start}
{environment}Restart=on-failure

[Install]
WantedBy=default.target
"#
            ),
            Agent::Tray => format!(
                r#"[Unit]
Description=Banshee menu bar icon
After=graphical-session.target
PartOf=graphical-session.target

[Service]
ExecStart={exec_start}
{environment}Restart=on-failure

[Install]
WantedBy=graphical-session.target
"#
            ),
        }
    }

    pub fn install(agent: Agent) -> Result<String, BansheeError> {
        let unit =
            unit_path(agent).ok_or_else(|| BansheeError::Other("config dir not found".into()))?;
        // No log paths: systemd captures stdout/stderr into the journal.
        banshee_common::utils::write_atomically(
            &unit,
            content(
                agent,
                &exec_start(agent)?,
                super::supervised_level().as_deref(),
            )
            .as_bytes(),
            None,
        )?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", unit_name(agent)])?;
        // restart, not start: a reinstall must hand over to the new binary
        systemctl(&["restart", unit_name(agent)])?;
        let service = unit_name(agent).trim_end_matches(".service");
        Ok(format!("journalctl --user -u {service} -f"))
    }

    pub fn uninstall(agent: Agent) -> Result<bool, BansheeError> {
        let _ = systemctl(&["disable", "--now", unit_name(agent)]);
        match unit_path(agent) {
            Some(unit) if unit.exists() => {
                std::fs::remove_file(&unit)?;
                let _ = systemctl(&["daemon-reload"]);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn systemctl(args: &[&str]) -> Result<(), BansheeError> {
        let output = Command::new(SYSTEMCTL).arg("--user").args(args).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(BansheeError::Other(format!(
                "systemctl --user {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }
}

#[cfg(target_os = "linux")]
pub use systemd::{install as start, service_file_path, uninstall};

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod unsupported {
    use std::path::PathBuf;

    use super::Agent;

    use banshee_common::error::BansheeError;

    fn unsupported() -> BansheeError {
        BansheeError::Other("service management is not supported on this platform yet".into())
    }

    pub fn service_file_path() -> Option<PathBuf> {
        None
    }

    pub fn install(_agent: Agent) -> Result<String, BansheeError> {
        Err(unsupported())
    }

    pub fn uninstall(_agent: Agent) -> Result<bool, BansheeError> {
        Err(unsupported())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub use unsupported::{install as start, service_file_path, uninstall};

#[cfg(test)]
mod sibling_tests {
    use banshee_common::utils::sibling;
    use std::path::PathBuf;

    // Builds a scratch directory holding a bundle layout for the test to use.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("banshee-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Banshee.app/Contents/MacOS")).unwrap();
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        dir
    }

    #[test]
    fn a_sibling_resolves_through_a_symlinked_cli() {
        let dir = scratch("sibling");
        let real = dir.join("Banshee.app/Contents/MacOS/banshee");
        let tray = dir.join("Banshee.app/Contents/MacOS/banshee-tray");
        std::fs::write(&real, "").unwrap();
        std::fs::write(&tray, "").unwrap();
        let link = dir.join("bin/banshee");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let found = sibling(&link, "banshee-tray").unwrap();

        assert_eq!(
            std::fs::canonicalize(&found).unwrap(),
            std::fs::canonicalize(&tray).unwrap(),
            "the lookup must land inside the bundle, not beside the symlink"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_sibling_names_itself() {
        let dir = scratch("missing");
        let real = dir.join("Banshee.app/Contents/MacOS/banshee");
        std::fs::write(&real, "").unwrap();

        let error = sibling(&real, "banshee-tray").expect_err("a missing tray must not succeed");

        assert!(
            error.to_string().contains("banshee-tray"),
            "unhelpful error: {error}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tool_tests {
    #[test]
    fn the_launcher_is_where_a_supervised_daemon_finds_it() {
        crate::test_support::tool_is_installed(super::login_items::LAUNCHCTL);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tool_tests {
    #[test]
    fn the_service_manager_is_where_a_supervised_daemon_finds_it() {
        crate::test_support::tool_is_installed(super::systemd::SYSTEMCTL);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod login_item_tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use banshee_common::utils::{LEGACY_AGENTS, Registration};

    use banshee_common::error::BansheeError;

    use super::Agent;
    use super::login_items::{
        awaits_approval, migrate_legacy, outcome, ready_to_register, registration_in,
    };

    struct TempHome(PathBuf);
    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_home(name: &str) -> TempHome {
        let home = std::env::temp_dir().join(format!("banshee-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Library/LaunchAgents")).unwrap();
        TempHome(home)
    }

    #[test]
    fn migration_removes_each_legacy_plist_and_boots_it_out() {
        let home = temp_home("migrate-legacy");
        let agents = home.0.join("Library/LaunchAgents");
        let plists: Vec<PathBuf> = LEGACY_AGENTS
            .iter()
            .map(|label| agents.join(format!("{label}.plist")))
            .collect();
        for plist in &plists {
            std::fs::write(plist, "<plist/>").unwrap();
        }
        let current = agents.join("com.banshee.app.daemon.plist");
        std::fs::write(&current, "<plist/>").unwrap();
        let booted = RefCell::new(Vec::new());

        let removed = migrate_legacy(&home.0, |label| booted.borrow_mut().push(label.to_string()))
            .expect("both plists go");

        assert_eq!(booted.into_inner(), LEGACY_AGENTS);
        assert_eq!(removed, plists);
        assert!(plists.iter().all(|plist| !plist.exists()));
        assert!(current.exists(), "only a legacy plist goes");
    }

    #[test]
    fn migration_with_nothing_legacy_does_nothing() {
        let home = temp_home("migrate-nothing");
        let booted = RefCell::new(Vec::<String>::new());

        let removed = migrate_legacy(&home.0, |label| booted.borrow_mut().push(label.to_string()))
            .expect("nothing to fail on");

        assert!(removed.is_empty());
        assert!(booted.into_inner().is_empty(), "nothing to boot out");
    }

    fn legacy_plists(home: &TempHome) -> Vec<PathBuf> {
        let plists: Vec<PathBuf> = LEGACY_AGENTS
            .iter()
            .map(|label| home.0.join(format!("Library/LaunchAgents/{label}.plist")))
            .collect();
        for plist in &plists {
            std::fs::write(plist, "<plist/>").unwrap();
        }
        plists
    }

    #[test]
    fn a_missing_app_leaves_the_legacy_jobs_alone() {
        let home = temp_home("migrate-no-app");
        let plists = legacy_plists(&home);
        let booted = RefCell::new(Vec::<String>::new());

        let refused = ready_to_register(
            &home.0,
            || Err(BansheeError::Other("banshee-app not found".into())),
            |label| booted.borrow_mut().push(label.to_string()),
        );

        assert!(refused.is_err());
        assert!(booted.into_inner().is_empty(), "nothing is booted out");
        assert!(plists.iter().all(|plist| plist.exists()));
    }

    #[test]
    fn a_found_app_migrates_before_it_registers() {
        let home = temp_home("migrate-app");
        let plists = legacy_plists(&home);
        let app = PathBuf::from("/Applications/Banshee.app/Contents/MacOS/banshee-app");

        let found = ready_to_register(&home.0, || Ok(app.clone()), |_| {});

        assert_eq!(found.expect("the app is found"), app);
        assert!(plists.iter().all(|plist| !plist.exists()));
    }

    #[test]
    fn a_plist_that_cannot_be_deleted_is_an_error_naming_it() {
        let home = temp_home("migrate-stuck");
        let stuck = home
            .0
            .join(format!("Library/LaunchAgents/{}.plist", LEGACY_AGENTS[0]));
        std::fs::create_dir_all(stuck.join("inside")).unwrap();

        let error = migrate_legacy(&home.0, |_| {}).expect_err("a directory is no file to delete");

        assert!(
            error.to_string().contains(&*stuck.to_string_lossy()),
            "{error}"
        );
        assert!(stuck.exists());
    }

    #[test]
    fn a_line_on_success_is_the_state() {
        assert_eq!(
            outcome(
                true,
                "com.banshee.app.daemon enabled\n",
                "",
                "com.banshee.app.daemon"
            )
            .expect("a state"),
            Registration::Enabled
        );
    }

    #[test]
    fn a_switched_off_agent_is_a_state_not_an_error() {
        assert_eq!(
            outcome(
                true,
                "com.banshee.app.daemon requires-approval\n",
                "",
                "com.banshee.app.daemon"
            )
            .expect("exit 0 with a line"),
            Registration::RequiresApproval
        );
    }

    #[test]
    fn an_agent_error_carries_what_the_app_said() {
        let error = outcome(
            false,
            "",
            "com.banshee.app.daemon: Operation not permitted\n",
            "com.banshee.app.daemon",
        )
        .expect_err("exit 1 with no line");
        assert!(
            error.to_string().contains("Operation not permitted"),
            "{error}"
        );
    }

    #[test]
    fn a_failed_run_is_an_error_even_with_a_line() {
        let error = outcome(
            false,
            "com.banshee.app.daemon enabled\n",
            "com.banshee.app.tray: Operation not permitted\n",
            "com.banshee.app.daemon",
        )
        .expect_err("exit 1 fails the run");
        assert!(
            error.to_string().contains("Operation not permitted"),
            "{error}"
        );
    }

    #[test]
    fn a_usage_error_carries_the_usage() {
        let error = outcome(
            false,
            "",
            "usage: banshee-app --login-items <register|unregister|status> [daemon|tray|all]\n",
            "com.banshee.app.daemon",
        )
        .expect_err("exit 2");
        assert!(error.to_string().contains("usage: banshee-app"), "{error}");
    }

    #[test]
    fn a_success_with_no_line_says_no_state_was_printed() {
        let error = outcome(true, "", "", "com.banshee.app.tray").expect_err("no line");
        assert_eq!(
            error.to_string(),
            "banshee-app printed no state for com.banshee.app.tray"
        );
    }

    #[test]
    fn a_registration_that_needs_approval_names_login_items() {
        for (agent, command) in [
            (Agent::Daemon, "banshee start"),
            (Agent::Tray, "banshee tray"),
        ] {
            let message = awaits_approval(agent);
            assert!(
                message.contains("System Settings > General > Login Items"),
                "{message}"
            );
            assert!(message.contains(command), "{message}");
        }
    }

    #[test]
    fn the_line_for_the_asked_label_gives_its_state() {
        let stdout = "com.banshee.app.daemon requires-approval\ncom.banshee.app.tray enabled\n";

        assert_eq!(
            registration_in(stdout, "com.banshee.app.daemon"),
            Some(Registration::RequiresApproval)
        );
        assert_eq!(
            registration_in(stdout, "com.banshee.app.tray"),
            Some(Registration::Enabled)
        );
    }

    #[test]
    fn no_line_or_an_unknown_word_gives_no_state() {
        assert_eq!(registration_in("", "com.banshee.app.daemon"), None);
        assert_eq!(
            registration_in(
                "com.banshee.app.daemon.extra enabled\n",
                "com.banshee.app.daemon"
            ),
            None
        );
        assert_eq!(
            registration_in("com.banshee.app.daemon on\n", "com.banshee.app.daemon"),
            None
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
mod service_file_tests {
    use super::Agent;

    #[test]
    fn the_level_the_installer_ran_with_reaches_the_supervised_daemon() {
        let unit = super::systemd::content(Agent::Daemon, "/usr/bin/banshee serve", Some("debug"));
        assert!(unit.contains("Environment=BANSHEE_LOG=debug"), "{unit}");
    }

    #[test]
    fn no_level_writes_no_environment() {
        let unit = super::systemd::content(Agent::Daemon, "/usr/bin/banshee serve", None);
        assert!(!unit.contains("Environment="), "{unit}");
    }
}
