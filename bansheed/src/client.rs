use std::fmt;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStringExt as _;
use std::path::{Path, PathBuf};

use crate::connect::SHIM_NAME;

pub struct Process {
    pub name: String,
    pub path: Option<PathBuf>,
    pub parent: u32,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Process {
    fn new(name: String, path: Option<PathBuf>, parent: u32) -> Option<Process> {
        (!name.is_empty()).then_some(Process { name, path, parent })
    }
}

#[cfg(target_os = "macos")]
pub fn process(pid: u32) -> Option<Process> {
    let id = libc::c_int::try_from(pid).ok().filter(|id| *id > 0)?;

    // SAFETY: proc_bsdshortinfo holds only integers and a byte array, so all
    // zero bytes are a valid value.
    let mut info: libc::proc_bsdshortinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdshortinfo>() as libc::c_int;
    // SAFETY: info is writable for the size passed, and proc_pidinfo writes at
    // most that many bytes.
    let written = unsafe {
        libc::proc_pidinfo(
            id,
            libc::PROC_PIDT_SHORTBSDINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if written <= 0 {
        return None;
    }

    let (name, path) = match executable(id) {
        Some(path) => (path.file_name()?.to_string_lossy().into_owned(), Some(path)),
        None => (short_name(id)?, None),
    };
    Process::new(name, path, info.pbsi_ppid)
}

#[cfg(target_os = "macos")]
fn executable(id: libc::c_int) -> Option<PathBuf> {
    let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: buffer is writable for the size passed, and proc_pidpath writes at
    // most that many bytes.
    let length = unsafe { libc::proc_pidpath(id, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    let length = usize::try_from(length).ok().filter(|length| *length > 0)?;
    buffer.truncate(length);
    Some(PathBuf::from(std::ffi::OsString::from_vec(buffer)))
}

#[cfg(target_os = "macos")]
fn short_name(id: libc::c_int) -> Option<String> {
    let mut buffer = [0u8; 64];
    // SAFETY: buffer is writable for the size passed, and proc_name writes at
    // most that many bytes.
    let length = unsafe { libc::proc_name(id, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    let length = usize::try_from(length).ok()?;
    Some(String::from_utf8_lossy(buffer.get(..length)?).into_owned())
}

#[cfg(target_os = "linux")]
pub fn process(pid: u32) -> Option<Process> {
    if pid == 0 {
        return None;
    }
    let dir = Path::new("/proc").join(pid.to_string());
    let exe = std::fs::read_link(dir.join("exe")).ok();
    let comm = match exe {
        Some(_) => None,
        None => std::fs::read_to_string(dir.join("comm")).ok(),
    };
    from_proc(
        exe,
        comm.as_deref(),
        std::fs::read_to_string(dir.join("status")).ok().as_deref(),
    )
}

#[cfg(target_os = "linux")]
fn from_proc(exe: Option<PathBuf>, comm: Option<&str>, status: Option<&str>) -> Option<Process> {
    let parent = status?
        .lines()
        .find_map(|line| line.strip_prefix("PPid:"))?
        .trim()
        .parse()
        .ok()?;
    let name = match (&exe, comm) {
        (Some(path), _) => named(path)?,
        (None, Some(comm)) => comm.trim_end().to_owned(),
        (None, None) => return None,
    };
    Process::new(name, exe, parent)
}

#[cfg(target_os = "linux")]
fn named(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy();
    let name = name.strip_suffix(" (deleted)").unwrap_or(&name);
    Some(name.to_owned())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn process(_pid: u32) -> Option<Process> {
    None
}

#[derive(Clone, Copy)]
pub struct Caller {
    pid: Option<u32>,
    read: fn(u32) -> Option<Process>,
}

impl Caller {
    pub fn of(pid: Option<u32>) -> Caller {
        Caller { pid, read: process }
    }

    #[cfg(test)]
    pub fn unread() -> Caller {
        Caller::of(None)
    }

    #[cfg(test)]
    pub const fn read_by(pid: u32, read: fn(u32) -> Option<Process>) -> Caller {
        Caller {
            pid: Some(pid),
            read,
        }
    }

    pub fn client(&self) -> Client {
        let Some(pid) = self.pid else {
            return Client::default();
        };
        let Some(peer) = (self.read)(pid) else {
            return Client {
                pid: Some(pid),
                ..Client::default()
            };
        };
        if peer.name != SHIM_NAME {
            return Client {
                name: Some(peer.name),
                pid: Some(pid),
                path: peer.path,
                ..Client::default()
            };
        }
        let agent = Some(peer.parent).filter(|parent| *parent > 1);
        match agent.and_then(|parent| (self.read)(parent).map(|process| (parent, process))) {
            Some((parent, process)) => Client {
                name: Some(process.name),
                agent,
                pid: Some(parent),
                path: process.path,
                shim: true,
            },
            None => Client {
                agent,
                pid: Some(pid),
                shim: true,
                ..Client::default()
            },
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Client {
    pub name: Option<String>,
    pub agent: Option<u32>,
    pid: Option<u32>,
    path: Option<PathBuf>,
    shim: bool,
}

impl fmt::Display for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(pid) = self.pid else {
            return f.write_str("a caller with no process ID");
        };
        if let Some(name) = &self.name {
            write!(f, "{name}, ")?;
        }
        write!(f, "process {pid}")?;
        if let Some(path) = &self.path {
            write!(f, ", {}", tilde(path, dirs::home_dir().as_deref()))?;
        }
        if self.shim {
            write!(f, ", through {SHIM_NAME}")?;
        }
        Ok(())
    }
}

fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(pid: u32) -> Option<Process> {
        let shim = "/Applications/Banshee.app/Contents/MacOS/banshee-mcp-shim";
        let claude =
            "/Users/someone/Library/Application Support/Claude/claude.app/Contents/MacOS/claude";
        let (path, parent) = match pid {
            53872 => (shim, 53482),
            53482 => (claude, 53481),
            900 => ("/usr/local/bin/banshee", 1),
            700 => (shim, 1),
            600 => (shim, 601),
            _ => return None,
        };
        let path = PathBuf::from(path);
        Some(Process {
            name: path.file_name()?.to_string_lossy().into_owned(),
            path: Some(path),
            parent,
        })
    }

    #[test]
    fn a_shim_names_its_parent_and_gives_the_agent() {
        let client = Caller::read_by(53872, table).client();
        assert_eq!(client.name.as_deref(), Some("claude"));
        assert_eq!(client.agent, Some(53482));
    }

    #[test]
    fn any_other_caller_names_itself_and_is_no_agent() {
        let client = Caller::read_by(900, table).client();
        assert_eq!(
            (client.name.as_deref(), client.agent),
            (Some("banshee"), None)
        );
    }

    #[test]
    fn a_shim_whose_agent_is_gone_has_no_name() {
        let client = Caller::read_by(700, table).client();
        assert_eq!((client.name, client.agent), (None, None));
    }

    #[test]
    fn a_shim_whose_parent_does_not_read_has_no_name_and_still_counts_for_that_agent() {
        let client = Caller::read_by(600, table).client();
        assert_eq!((client.name, client.agent), (None, Some(601)));
    }

    #[test]
    fn a_caller_with_no_process_id_or_no_read_has_no_name() {
        assert_eq!(Caller::unread().client().name, None);
        assert_eq!(Caller::read_by(4, table).client().name, None);
    }

    #[test]
    fn the_log_form_holds_the_name_the_process_the_path_and_the_shim() {
        assert_eq!(
            Caller::read_by(900, table).client().to_string(),
            "banshee, process 900, /usr/local/bin/banshee"
        );
        let shim = Caller::read_by(53872, table).client().to_string();
        assert!(shim.starts_with("claude, process 53482, "), "{shim}");
        assert!(shim.ends_with(", through banshee-mcp-shim"), "{shim}");
        assert_eq!(Caller::read_by(4, table).client().to_string(), "process 4");
        assert_eq!(
            Caller::unread().client().to_string(),
            "a caller with no process ID"
        );
        assert_eq!(
            Caller::read_by(700, table).client().to_string(),
            "process 700, through banshee-mcp-shim"
        );
    }

    #[test]
    fn the_home_folder_is_written_as_a_tilde() {
        let home = Path::new("/Users/someone");
        assert_eq!(
            tilde(Path::new("/Users/someone/bin/claude"), Some(home)),
            "~/bin/claude"
        );
        assert_eq!(
            tilde(Path::new("/usr/bin/claude"), Some(home)),
            "/usr/bin/claude"
        );
    }

    #[test]
    fn this_process_reads_as_its_own_executable_and_parent() {
        let me = process(std::process::id()).expect("a live process reads");
        let exe = std::env::current_exe().unwrap();
        assert_eq!(me.name, exe.file_name().unwrap().to_string_lossy());
        assert_eq!(me.parent, std::os::unix::process::parent_id());
    }

    #[test]
    fn process_zero_and_a_process_that_is_gone_read_as_nothing() {
        assert!(process(0).is_none());
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(process(pid).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_process_whose_file_was_deleted_still_reads_by_its_name() {
        let dir = std::env::temp_dir().join(format!("banshee-client-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join(SHIM_NAME);
        std::fs::copy("/bin/sleep", &copy).unwrap();
        let mut child = std::process::Command::new(&copy).arg("30").spawn().unwrap();
        let pid = child.id();
        std::fs::remove_file(&copy).unwrap();
        let read = process(pid);
        child.kill().unwrap();
        child.wait().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        let read = read.expect("a deleted executable still reads");
        assert_eq!(read.name, SHIM_NAME);
        assert!(read.path.is_none());
        assert_eq!(read.parent, std::process::id());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_replaced_binary_loses_the_deleted_suffix() {
        assert_eq!(
            named(Path::new("/usr/bin/claude (deleted)")).as_deref(),
            Some("claude")
        );
        assert_eq!(
            named(Path::new("/usr/bin/claude")).as_deref(),
            Some("claude")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_process_whose_name_would_be_empty_reads_as_nothing() {
        let status = Some("PPid:\t41\n");
        assert!(from_proc(None, Some("\n"), status).is_none());
        assert!(from_proc(Some(PathBuf::from("/")), Some("claude"), status).is_none());
        assert!(named(Path::new("/")).is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_unreadable_exe_link_falls_back_to_comm() {
        let read = from_proc(None, Some("claude\n"), Some("Name:\tclaude\nPPid:\t41\n")).unwrap();
        assert_eq!(
            (read.name.as_str(), read.path, read.parent),
            ("claude", None, 41)
        );
    }
}
