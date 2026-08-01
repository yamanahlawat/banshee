// Start-at-login behind a neutral surface: launchd on macOS, systemd on Linux.

#[cfg(target_os = "macos")]
mod launchd {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use banshee_common::error::BansheeError;

    const LABEL: &str = "com.banshee.daemon";

    pub fn service_file_path() -> Option<PathBuf> {
        Some(agent_path(&dirs::home_dir()?))
    }

    fn agent_path(home: &Path) -> PathBuf {
        home.join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"))
    }

    fn home_dir() -> Result<PathBuf, BansheeError> {
        dirs::home_dir().ok_or_else(|| BansheeError::Other("home dir not found".into()))
    }

    pub fn install() -> Result<(), BansheeError> {
        let home = home_dir()?;
        let plist = agent_path(&home);
        let binary = std::env::current_exe()?;
        let log = home.join(".banshee").join("daemon.log");
        std::fs::create_dir_all(home.join(".banshee"))?;
        if let Some(dir) = plist.parent() {
            std::fs::create_dir_all(dir)?;
        }

        let content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>serve</string>
    </array>
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
            binary.display(),
            log = log.display(),
        );

        // Reinstall must be idempotent: unload any previous copy before loading
        let _ = launchctl(&["bootout", &format!("gui/{}/{LABEL}", uid())]);
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
        result?;
        println!("Started {LABEL}: the daemon runs at login and restarts on crash.");
        println!("Logs: {}", log.display());
        println!("A daemon already running in a terminal keeps the socket; stop it to hand over.");
        Ok(())
    }

    pub fn uninstall() -> Result<(), BansheeError> {
        let plist = agent_path(&home_dir()?);
        let _ = launchctl(&["bootout", &format!("gui/{}/{LABEL}", uid())]);
        if plist.exists() {
            std::fs::remove_file(&plist)?;
            println!("Removed {LABEL}.");
        } else {
            println!("No launch agent installed.");
        }
        Ok(())
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
    use std::process::Command;

    use banshee_common::error::BansheeError;

    const UNIT: &str = "banshee.service";

    pub fn service_file_path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("systemd/user").join(UNIT))
    }

    pub fn install() -> Result<(), BansheeError> {
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
        println!("Started {UNIT}: the daemon runs at login and restarts on crash.");
        println!("Logs: journalctl --user -u banshee -f");
        println!("A daemon already running in a terminal keeps the socket; stop it to hand over.");
        Ok(())
    }

    pub fn uninstall() -> Result<(), BansheeError> {
        let _ = systemctl(&["disable", "--now", UNIT]);
        match service_file_path() {
            Some(unit) if unit.exists() => {
                std::fs::remove_file(&unit)?;
                let _ = systemctl(&["daemon-reload"]);
                println!("Removed {UNIT}.");
            }
            _ => println!("No service installed."),
        }
        Ok(())
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

    use banshee_common::error::BansheeError;

    fn unsupported() -> BansheeError {
        BansheeError::Other("service management is not supported on this platform yet".into())
    }

    pub fn service_file_path() -> Option<PathBuf> {
        None
    }

    pub fn install() -> Result<(), BansheeError> {
        Err(unsupported())
    }

    pub fn uninstall() -> Result<(), BansheeError> {
        Err(unsupported())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub use unsupported::{install, service_file_path, uninstall};
