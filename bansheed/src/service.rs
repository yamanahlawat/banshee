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
fn supervised_level() -> Option<String> {
    std::env::var("BANSHEE_LOG")
        .ok()
        .filter(|level| !level.trim().is_empty())
}

pub(crate) fn home_dir() -> Result<PathBuf, BansheeError> {
    dirs::home_dir().ok_or_else(|| BansheeError::Other("home dir not found".into()))
}

#[cfg(target_os = "macos")]
mod launchd {
    pub(super) const LAUNCHCTL: &str = "/bin/launchctl";

    use std::path::{Path, PathBuf};
    use std::process::Command;

    use banshee_common::error::BansheeError;

    use banshee_common::utils::{DAEMON_AGENT, TRAY_AGENT, launchd_target, uid};

    use super::{Agent, home_dir};
    use banshee_common::utils::sibling;

    fn label(agent: Agent) -> &'static str {
        match agent {
            Agent::Daemon => DAEMON_AGENT,
            Agent::Tray => TRAY_AGENT,
        }
    }

    pub fn service_file_path() -> Option<PathBuf> {
        Some(agent_path(&dirs::home_dir()?, label(Agent::Daemon)))
    }

    fn agent_path(home: &Path, label: &str) -> PathBuf {
        home.join("Library/LaunchAgents")
            .join(format!("{label}.plist"))
    }

    /// One plist shape for both agents. `KeepAlive` fires only on failure, so
    /// the tray's Quit item is not undone by launchd starting it again.
    pub(super) fn plist(
        label: &str,
        arguments: &[String],
        log: &Path,
        level: Option<&str>,
    ) -> String {
        let arguments: String = arguments
            .iter()
            .map(|argument| format!("        <string>{argument}</string>\n"))
            .collect();
        let environment = level.map_or(String::new(), |level| {
            format!(
                "    <key>EnvironmentVariables</key>\n    <dict>\n        <key>BANSHEE_LOG</key>\n        <string>{level}</string>\n    </dict>\n"
            )
        });

        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
{arguments}    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
{environment}    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#,
            log = log.display(),
        )
    }

    fn write_agent(label: &str, arguments: &[String], log: &Path) -> Result<(), BansheeError> {
        let home = home_dir()?;
        let file = agent_path(&home, label);
        std::fs::create_dir_all(home.join(".banshee"))?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }

        let content = plist(label, arguments, log, super::supervised_level().as_deref());

        // Install means make this binary the one that runs, so a live job is
        // torn down even when the plist is identical.
        let _ = launchctl(&["bootout", &launchd_target(label)]);
        banshee_common::utils::write_atomically(&file, content.as_bytes(), None)?;
        // bootout is asynchronous and bootstrap fails while the old job is
        // still tearing down, so retry the bootstrap itself
        let mut result = Ok(());
        for _ in 0..50 {
            result = launchctl(&[
                "bootstrap",
                &format!("gui/{}", uid()),
                &file.to_string_lossy(),
            ]);
            if result.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        result
    }

    fn remove_agent(label: &str) -> Result<bool, BansheeError> {
        let plist = agent_path(&home_dir()?, label);
        let _ = launchctl(&["bootout", &launchd_target(label)]);
        if plist.exists() {
            std::fs::remove_file(&plist)?;
            return Ok(true);
        }
        Ok(false)
    }

    // The daemon is this binary; the tray ships beside it, so one install
    // moves the pair together
    fn program(agent: Agent) -> Result<Vec<String>, BansheeError> {
        let binary = std::env::current_exe()?;
        match agent {
            Agent::Daemon => {
                let real = std::fs::canonicalize(&binary)?;
                Ok(vec![real.display().to_string(), "serve".to_string()])
            }
            Agent::Tray => Ok(vec![
                sibling(&binary, "banshee-tray")?.display().to_string(),
            ]),
        }
    }

    /// Returns where this platform keeps that agent's log.
    pub fn install(agent: Agent) -> Result<String, BansheeError> {
        let log = home_dir()?
            .join(".banshee")
            .join(format!("{}.log", agent.name()));
        write_agent(label(agent), &program(agent)?, &log)?;
        Ok(log.display().to_string())
    }

    /// True when there was one to remove.
    pub fn uninstall(agent: Agent) -> Result<bool, BansheeError> {
        remove_agent(label(agent))
    }

    fn launchctl(args: &[&str]) -> Result<(), BansheeError> {
        // output() also swallows the noise from ignored bootout pre-cleans
        let output = Command::new(LAUNCHCTL).args(args).output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(BansheeError::Other(format!(
                "launchctl {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }
}

#[cfg(target_os = "macos")]
pub use launchd::{install, service_file_path, uninstall};

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
pub use systemd::{install, service_file_path, uninstall};

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
pub use unsupported::{install, service_file_path, uninstall};

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
        crate::test_support::tool_is_installed(super::launchd::LAUNCHCTL);
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
mod service_file_tests {
    use std::path::Path;

    #[test]
    fn the_level_the_installer_ran_with_reaches_the_supervised_daemon() {
        let written = super::launchd::plist(
            "com.banshee.daemon",
            &["/usr/local/bin/banshee".to_string(), "serve".to_string()],
            Path::new("/Users/someone/.banshee/daemon.log"),
            Some("debug"),
        );
        assert!(
            written.contains("<key>EnvironmentVariables</key>"),
            "{written}"
        );
        assert!(written.contains("<key>BANSHEE_LOG</key>"), "{written}");
        assert!(written.contains("<string>debug</string>"), "{written}");
    }

    #[test]
    fn no_level_writes_no_environment() {
        let written = super::launchd::plist(
            "com.banshee.daemon",
            &["/usr/local/bin/banshee".to_string(), "serve".to_string()],
            Path::new("/Users/someone/.banshee/daemon.log"),
            None,
        );
        assert!(!written.contains("EnvironmentVariables"), "{written}");
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
