// Start-at-login behind a neutral surface: launchd on macOS, systemd on Linux.

/// What can be set to start at login. The platform arms decide what they can
/// honour, so a new entry is one variant rather than a new pair of functions.
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

#[cfg(target_os = "macos")]
mod launchd {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use banshee_common::error::BansheeError;

    use super::Agent;

    fn label(agent: Agent) -> &'static str {
        match agent {
            Agent::Daemon => "com.banshee.daemon",
            Agent::Tray => "com.banshee.tray",
        }
    }

    pub fn service_file_path() -> Option<PathBuf> {
        Some(agent_path(&dirs::home_dir()?, label(Agent::Daemon)))
    }

    fn agent_path(home: &Path, label: &str) -> PathBuf {
        home.join("Library/LaunchAgents")
            .join(format!("{label}.plist"))
    }

    fn home_dir() -> Result<PathBuf, BansheeError> {
        dirs::home_dir().ok_or_else(|| BansheeError::Other("home dir not found".into()))
    }

    /// One plist shape for both agents. `KeepAlive` fires only on failure, so
    /// the tray's Quit item is not undone by launchd starting it again.
    fn write_agent(label: &str, arguments: &[String], log: &Path) -> Result<(), BansheeError> {
        let home = home_dir()?;
        let plist = agent_path(&home, label);
        std::fs::create_dir_all(home.join(".banshee"))?;
        if let Some(dir) = plist.parent() {
            std::fs::create_dir_all(dir)?;
        }

        let arguments: String = arguments
            .iter()
            .map(|argument| format!("        <string>{argument}</string>\n"))
            .collect();

        let content = format!(
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
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#,
            log = log.display(),
        );

        // Reinstall must be idempotent: unload any previous copy before loading
        let _ = launchctl(&["bootout", &format!("gui/{}/{label}", uid())]);
        std::fs::write(&plist, content)?;
        // bootout is asynchronous and bootstrap fails while the old job is
        // still tearing down, so retry the bootstrap itself
        let mut result = Ok(());
        for _ in 0..50 {
            result = launchctl(&[
                "bootstrap",
                &format!("gui/{}", uid()),
                &plist.to_string_lossy(),
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
        let _ = launchctl(&["bootout", &format!("gui/{}/{label}", uid())]);
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
            Agent::Daemon => Ok(vec![binary.display().to_string(), "serve".to_string()]),
            Agent::Tray => {
                let tray = binary
                    .parent()
                    .ok_or_else(|| BansheeError::Other("banshee is not inside a directory".into()))?
                    .join("banshee-tray");
                if !tray.exists() {
                    return Err(BansheeError::Other(format!(
                        "{} not found; reinstall so the tray ships beside the CLI",
                        tray.display()
                    )));
                }
                Ok(vec![tray.display().to_string()])
            }
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
        let output = Command::new("launchctl").args(args).output()?;
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

    fn uid() -> u32 {
        unsafe extern "C" {
            fn getuid() -> u32;
        }
        unsafe { getuid() }
    }
}

#[cfg(target_os = "macos")]
pub use launchd::{install, service_file_path, uninstall};

#[cfg(target_os = "linux")]
mod systemd {
    use std::path::PathBuf;

    use super::Agent;
    use std::process::Command;

    use banshee_common::error::BansheeError;

    const UNIT: &str = "banshee.service";

    pub fn service_file_path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("systemd/user").join(UNIT))
    }

    pub fn install(agent: Agent) -> Result<String, BansheeError> {
        if agent != Agent::Daemon {
            return Err(BansheeError::Other(
                "the menu bar icon is macOS only; here use: banshee watch --waybar".into(),
            ));
        }
        let unit = service_file_path()
            .ok_or_else(|| BansheeError::Other("config dir not found".into()))?;
        let binary = std::env::current_exe()?;
        if let Some(dir) = unit.parent() {
            std::fs::create_dir_all(dir)?;
        }

        // No log paths: systemd captures stdout/stderr into the journal.
        // After=pipewire.service so the mic exists before the daemon opens it
        let content = format!(
            r#"[Unit]
Description=Banshee voice daemon
After=pipewire.service

[Service]
ExecStart="{}" serve
Restart=on-failure

[Install]
WantedBy=default.target
"#,
            binary.display()
        );

        std::fs::write(&unit, content)?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", UNIT])?;
        // restart, not start: a reinstall must hand over to the new binary
        systemctl(&["restart", UNIT])?;
        Ok("journalctl --user -u banshee -f".to_string())
    }

    pub fn uninstall(agent: Agent) -> Result<bool, BansheeError> {
        if agent != Agent::Daemon {
            return Ok(false);
        }
        let _ = systemctl(&["disable", "--now", UNIT]);
        match service_file_path() {
            Some(unit) if unit.exists() => {
                std::fs::remove_file(&unit)?;
                let _ = systemctl(&["daemon-reload"]);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn systemctl(args: &[&str]) -> Result<(), BansheeError> {
        let output = Command::new("systemctl")
            .arg("--user")
            .args(args)
            .output()?;
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
