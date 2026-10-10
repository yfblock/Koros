//! Minimal procfs for korix: dynamic pseudo-files under `/proc`.
//!
//! Serves the entries busybox `ps`/`free`/`uptime`/`cat /proc/version` need:
//! `meminfo`, `cpuinfo`, `uptime`, `version`, `loadavg`, `mounts`, `stat`,
//! and one `<pid>/` directory per live process exposing `stat`, `status`,
//! and `cmdline`.  All content is generated at read time from kernel state —
//! there is no backing storage and every write is rejected.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

use kor::{FileType, FsError, FsInfo, INode, Metadata, SuperBlock};

/// What a node generates when read.
enum Kind {
    /// Fixed top-level directory (`/proc`).
    Root,
    /// Per-process directory (`/proc/<pid>`).
    Process(usize),
    /// `/proc/sys` tunables directory.
    SysDir,
    /// `/proc/sys/kernel` tunables directory.
    SysKernel,
    /// Regular pseudo-file rendered by a generator function.
    File(fn() -> String),
}

/// One procfs node; content is produced on demand from kernel state.
pub struct ProcNode {
    kind: Kind,
    /// Stable name for inode hashing and static file identity.
    name: &'static str,
    /// Late-bound pid for per-process files (0 for the rest).
    pid: usize,
}

/// Placeholder for per-process files — the real content comes from the
/// pid-dispatching branch of read_at; this only satisfies Kind::File.
fn placeholder_generator() -> String {
    String::new()
}

impl ProcNode {
    const ROOT_MODE: u32 = 0o555;
    const FILE_MODE: u32 = 0o444;

    fn dir(kind: Kind) -> Arc<Self> {
        Arc::new(Self {
            kind,
            name: "dir",
            pid: 0,
        })
    }

    fn file(name: &'static str, generator: fn() -> String) -> Arc<Self> {
        Arc::new(Self {
            kind: Kind::File(generator),
            name,
            pid: 0,
        })
    }

    fn process_file(pid: usize, base: &'static str) -> Arc<Self> {
        Arc::new(Self {
            kind: Kind::File(placeholder_generator),
            name: base,
            pid,
        })
    }
    /// Stable inode number: a hash of the identity keeps getdents64 output
    fn stable_ino(text: &str) -> u32 {
        // FNV-1a over the name, biased away from 0 and 1 (root conventions).
        let mut hash: u32 = 0x811C_9DC5;
        for byte in text.bytes() {
            hash ^= byte as u32;
            hash = hash.wrapping_mul(0x0100_0193);
        }
        hash | 0x8000_0000
    }

    fn render(&self) -> String {
        match &self.kind {
            Kind::File(generator) => generator(),
            Kind::Root | Kind::Process(_) | Kind::SysDir | Kind::SysKernel => String::new(),
        }
    }
}

fn meminfo() -> String {
    let (total_kib, free_kib) = crate::procfs::memory_report();
    format!(
        "MemTotal:       {total} kB\nMemFree:        {free} kB\nBuffers:          0 kB\n\
         Cached:          0 kB\nSwapTotal:       0 kB\nSwapFree:        0 kB\n",
        total = total_kib,
        free = free_kib,
    )
}

fn uptime() -> String {
    let seconds = kor_arch::now().as_secs();
    format!("{seconds}.00 {seconds}.00\n")
}

fn version() -> String {
    "Linux version 6.1.0-korix (korix) riscv64\n".to_string()
}

fn loadavg() -> String {
    "0.00 0.00 0.00 1/16 1\n".to_string()
}

fn pid_max() -> String {
    "4194304\n".to_string()
}

fn threads_max() -> String {
    "62904\n".to_string()
}

fn ngroups_max() -> String {
    "65536\n".to_string()
}

fn osrelease() -> String {
    "6.12.0-korix\n".to_string()
}

fn ostype() -> String {
    "Linux\n".to_string()
}

fn cpuinfo() -> String {
    let harts = kor::smp::online_count();
    let mut out = String::new();
    for hart in 0..harts {
        out.push_str(&format!(
            "processor\t: {hart}\nhart\t\t: {hart}\nisa\t\t: rv64imafdcsu\nmmu\t\t: sv39\n\n"
        ));
    }
    out
}

fn proc_stat() -> String {
    let ticks = kor_arch::ticks();
    let harts = kor::smp::online_count();
    let mut out = format!(
        "cpu {} 0 {} 0 {} 0 0 0 0 0 0\n",
        ticks,
        ticks,
        ticks * harts as u64
    );
    for hart in 0..harts {
        out.push_str(&format!("cpu{hart} 0 0 0 0 0 0 0 0 0 0\n"));
    }
    out.push_str("intr 0\nctxt 0\nbtime 0\nprocesses 1\nprocs_running 1\nprocs_blocked 0\n");
    out
}

fn proc_mounts() -> String {
    let mut out = String::new();
    for point in crate::MOUNTS.lock().mount_points() {
        out.push_str(&format!("/dev/root {point} ext2 rw 0 0\n"));
    }
    out
}

/// Snapshot of the scheduler's live process table: (pid, name, state, ppid).
fn process_table() -> Vec<(usize, [u8; 16], usize, usize)> {
    crate::sched::process_snapshot()
}

/// `/proc/<pid>/stat`: pid (comm) state ppid ... — the fields busybox ps reads.
fn process_stat(pid: usize) -> String {
    let Some(entry) = crate::sched::process_snapshot()
        .into_iter()
        .find(|p| p.0 == pid)
    else {
        return String::new();
    };
    let name = core::str::from_utf8(&entry.1)
        .unwrap_or("proc")
        .trim_end_matches('\0');
    let letter = match entry.2 {
        0 => 'R',
        1 => 'Z',
        _ => 'S',
    };
    format!(
        "{pid} ({name}) {letter} {ppid} {ppid} {ppid} 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
        ppid = entry.3,
    )
}

/// `/proc/<pid>/status`: human-readable process summary.
fn process_status(pid: usize) -> String {
    let Some(entry) = crate::sched::process_snapshot()
        .into_iter()
        .find(|p| p.0 == pid)
    else {
        return String::new();
    };
    let name = core::str::from_utf8(&entry.1)
        .unwrap_or("proc")
        .trim_end_matches('\0');
    let state_name = match entry.2 {
        0 => "R (running)",
        1 => "Z (zombie)",
        _ => "S (sleeping)",
    };
    format!(
        "Name:\t{name}\nState:\t{state_name}\nPid:\t{pid}\nPPid:\t{ppid}\nUid:\t0\t0\t0\t0\nGid:\t0\t0\t0\t0\n",
        ppid = entry.3,
    )
}

/// `/proc/<pid>/cmdline`: NUL-terminated argv, reconstructed from the name.
fn process_cmdline(pid: usize) -> String {
    let Some(entry) = crate::sched::process_snapshot()
        .into_iter()
        .find(|p| p.0 == pid)
    else {
        return String::new();
    };
    let name = core::str::from_utf8(&entry.1)
        .unwrap_or("proc")
        .trim_end_matches('\0');
    let mut out = String::from(name);
    out.push('\0');
    out
}

impl INode for ProcNode {
    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize, FsError> {
        let content = match (&self.kind, self.pid) {
            (Kind::File(generator), 0) => generator(),
            (Kind::File(_), pid) => match self.name {
                "stat" => process_stat(pid),
                "status" => process_status(pid),
                _ => process_cmdline(pid),
            },
            (Kind::Root | Kind::Process(_) | Kind::SysDir | Kind::SysKernel, _) => String::new(),
        };
        let bytes = content.as_bytes();
        if offset >= bytes.len() {
            return Ok(0);
        }
        let end = (offset + buf.len()).min(bytes.len());
        buf[..end - offset].copy_from_slice(&bytes[offset..end]);
        Ok(end - offset)
    }

    fn write_at(&self, _offset: usize, _buf: &[u8]) -> Result<usize, FsError> {
        Err(FsError::ReadOnly)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn INode>, FsError> {
        match &self.kind {
            Kind::Root => {
                if name == "self" {
                    let pid = crate::sched::current_pid().unwrap_or(1);
                    return Ok(ProcNode::dir(Kind::Process(pid)));
                }
                if let Some(pid) = name.parse::<usize>().ok().filter(|pid| *pid > 0) {
                    if process_table().iter().any(|p| p.0 == pid) {
                        return Ok(ProcNode::dir(Kind::Process(pid)));
                    }
                    return Err(FsError::NotFound);
                }
                match name {
                    "sys" => Ok(ProcNode::dir(Kind::SysDir)),
                    "meminfo" => Ok(ProcNode::file("meminfo", meminfo)),
                    "cpuinfo" => Ok(ProcNode::file("cpuinfo", cpuinfo)),
                    "uptime" => Ok(ProcNode::file("uptime", uptime)),
                    "version" => Ok(ProcNode::file("version", version)),
                    "loadavg" => Ok(ProcNode::file("loadavg", loadavg)),
                    "stat" => Ok(ProcNode::file("stat", proc_stat)),
                    "mounts" => Ok(ProcNode::file("mounts", proc_mounts)),
                    _ => Err(FsError::NotFound),
                }
            }
            Kind::SysDir => match name {
                "kernel" => Ok(ProcNode::dir(Kind::SysKernel)),
                _ => Err(FsError::NotFound),
            },
            Kind::SysKernel => match name {
                "pid_max" => Ok(ProcNode::file("pid_max", pid_max)),
                "threads-max" => Ok(ProcNode::file("threads-max", threads_max)),
                "ngroups_max" => Ok(ProcNode::file("ngroups_max", ngroups_max)),
                "osrelease" => Ok(ProcNode::file("osrelease", osrelease)),
                "ostype" => Ok(ProcNode::file("ostype", ostype)),
                _ => Err(FsError::NotFound),
            },
            Kind::Process(pid) => match name {
                "stat" => Ok(ProcNode::process_file(*pid, "stat")),
                "status" => Ok(ProcNode::process_file(*pid, "status")),
                "cmdline" => Ok(ProcNode::process_file(*pid, "cmdline")),
                _ => Err(FsError::NotFound),
            },
            Kind::File(_) => Err(FsError::NotDirectory),
        }
    }

    fn create(
        &self,
        _name: &str,
        _file_type: FileType,
        _mode: u32,
    ) -> Result<Arc<dyn INode>, FsError> {
        Err(FsError::ReadOnly)
    }

    fn unlink(&self, _name: &str) -> Result<(), FsError> {
        Err(FsError::ReadOnly)
    }

    fn mkdir(&self, _name: &str, _mode: u32) -> Result<Arc<dyn INode>, FsError> {
        Err(FsError::ReadOnly)
    }

    fn rmdir(&self, _name: &str) -> Result<(), FsError> {
        Err(FsError::ReadOnly)
    }

    fn getattr(&self) -> Result<Metadata, FsError> {
        let file_type = match self.kind {
            Kind::File(_) => FileType::Regular,
            Kind::Root | Kind::Process(_) | Kind::SysDir | Kind::SysKernel => FileType::Directory,
        };
        let mode = match file_type {
            FileType::Directory => Self::ROOT_MODE,
            _ => Self::FILE_MODE,
        };
        let size = match self.kind {
            Kind::File(_) => self.render().len(),
            _ => 0,
        };
        let now = kor_arch::now().as_secs();
        Ok(Metadata {
            size,
            mode,
            uid: 0,
            gid: 0,
            atime: now,
            mtime: now,
            ctime: now,
            file_type,
        })
    }

    fn setattr(&self, _metadata: &Metadata) -> Result<(), FsError> {
        Err(FsError::ReadOnly)
    }

    fn list(&self) -> Result<Vec<(String, u32)>, FsError> {
        match &self.kind {
            Kind::Root => {
                let mut entries: Vec<(String, u32)> = [
                    ("meminfo", meminfo as fn() -> String),
                    ("cpuinfo", cpuinfo as fn() -> String),
                    ("uptime", uptime as fn() -> String),
                    ("version", version as fn() -> String),
                    ("loadavg", loadavg as fn() -> String),
                    ("stat", proc_stat as fn() -> String),
                    ("mounts", proc_mounts as fn() -> String),
                ]
                .iter()
                .map(|(name, _)| ((*name).to_string(), Self::FILE_MODE))
                .collect();
                for process in process_table() {
                    entries.push((process.0.to_string(), Self::ROOT_MODE));
                }
                entries.push(("self".to_string(), Self::ROOT_MODE));
                entries.push(("sys".to_string(), Self::ROOT_MODE));
                Ok(entries)
            }
            Kind::SysDir => Ok(alloc::vec![("kernel".to_string(), Self::ROOT_MODE)]),
            Kind::SysKernel => Ok(alloc::vec![
                ("pid_max".to_string(), Self::FILE_MODE),
                ("threads-max".to_string(), Self::FILE_MODE),
                ("ngroups_max".to_string(), Self::FILE_MODE),
                ("osrelease".to_string(), Self::FILE_MODE),
                ("ostype".to_string(), Self::FILE_MODE),
            ]),
            Kind::Process(_) => Ok(alloc::vec![
                ("stat".to_string(), Self::FILE_MODE),
                ("status".to_string(), Self::FILE_MODE),
                ("cmdline".to_string(), Self::FILE_MODE),
            ]),
            Kind::File(_) => Err(FsError::NotDirectory),
        }
    }

    fn ino(&self) -> u32 {
        match &self.kind {
            Kind::Root => 1,
            Kind::Process(pid) => Self::stable_ino(&format!("proc{pid}")),
            Kind::SysDir => Self::stable_ino("sysdir"),
            Kind::SysKernel => Self::stable_ino("syskernel"),
            Kind::File(_) => Self::stable_ino(self.name),
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The procfs instance mounted at `/proc`.
pub struct ProcFs {
    root: Arc<ProcNode>,
}

impl ProcFs {
    /// Create a procfs instance whose root directory is fully dynamic.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            root: ProcNode::dir(Kind::Root),
        })
    }
}

impl SuperBlock for ProcFs {
    fn root_inode(&self) -> Arc<dyn INode> {
        self.root.clone()
    }

    fn sync(&self) -> Result<(), FsError> {
        Ok(())
    }

    fn info(&self) -> FsInfo {
        FsInfo {
            total_blocks: 0,
            free_blocks: 0,
            total_inodes: 0,
            free_inodes: 0,
            block_size: 0,
        }
    }

    fn read_only(&self) -> bool {
        true
    }
}

/// Total and free memory in KiB, sourced from the frame allocator.
pub fn memory_report() -> (usize, usize) {
    const PAGE_KIB: usize = 4;
    let free_frames = kor::frame::available_frames();
    let total_kib = crate::TOTAL_MEMORY_KIB.load(core::sync::atomic::Ordering::Relaxed);
    let free_kib = free_frames * PAGE_KIB;
    (total_kib, free_kib)
}

/// Mount procfs at `/proc`.
pub fn mount_procfs() {
    let _ = kor_fs::mount::mount(&crate::MOUNTS, "/proc", ProcFs::new());
}
