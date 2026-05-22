use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::process::{Command, Stdio};
use std::time::Duration;

pub fn spawn(
    cmd: String,
    args: Vec<String>,
    env: Vec<(OsString, OsString)>,
) -> Result<(), ()> {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // Remove evil environment variables of the devil
    command
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("LD_LIBRARY_PATH");

    command.envs(env);

    // Double-fork to run the executable.
    // Has to do with preventing zombie processes and such
    match unsafe { libc::fork() } {
        0 => {
            // child process
            unsafe {
                libc::setsid();
            }
            let _ = command.spawn();
            unsafe {
                libc::_exit(0);
            }
        }
        -1 => {
            // fork failed
            return Err(());
        }
        _ => { // parent process
        }
    }

    unsafe {
        libc::wait(std::ptr::null_mut());
    }

    Ok(())
}

// Terminate every process WaylandCraft launched, on JVM shutdown. spawn()
// setsid()s each child into its own process group, so a Steam/Proton/Xwayland
// app and everything it forked share one pgid; signalling the group catches
// the whole tree. A process counts as ours if its environ carries our
// WAYLAND_DISPLAY, or DISPLAY=:N for Xwayland clients. Returns the group count.
pub fn cleanup_display_processes(
    socket: &OsStr,
    xdisplay: Option<u32>,
) -> usize {
    let own_pgid = unsafe { libc::getpgid(0) };
    let groups = matching_process_groups(socket, xdisplay, own_pgid);
    if groups.is_empty() {
        return 0;
    }

    for pgid in &groups {
        unsafe {
            libc::kill(-pgid, libc::SIGTERM);
        }
    }

    std::thread::sleep(Duration::from_millis(750));

    // SIGKILL only groups that still have a matching process - a pgid could
    // have been reused by an unrelated process after the group exited.
    for pgid in &groups {
        if group_still_matches(*pgid, socket, xdisplay) {
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
    }

    groups.len()
}

// Walk /proc and collect the distinct process groups whose environ matches our
// display, skipping our own pid and our own process group.
fn matching_process_groups(
    socket: &OsStr,
    xdisplay: Option<u32>,
    own_pgid: libc::pid_t,
) -> HashSet<libc::pid_t> {
    let own_pid = unsafe { libc::getpid() };
    let mut groups = HashSet::new();

    let entries = match fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(_) => return groups,
    };

    for entry in entries.flatten() {
        let Some(pid) = parse_pid(&entry.file_name()) else {
            continue;
        };
        if pid == own_pid {
            continue;
        }

        let Ok(environ) = fs::read(entry.path().join("environ")) else {
            continue;
        };
        if !environ_matches(&environ, socket, xdisplay) {
            continue;
        }

        let pgid = unsafe { libc::getpgid(pid) };
        if pgid > 0 && pgid != own_pgid {
            groups.insert(pgid);
        }
    }

    groups
}

// True if any live process in the given group still matches our display.
fn group_still_matches(
    pgid: libc::pid_t,
    socket: &OsStr,
    xdisplay: Option<u32>,
) -> bool {
    let entries = match fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(_) => return false,
    };

    entries.flatten().any(|entry| {
        let Some(pid) = parse_pid(&entry.file_name()) else {
            return false;
        };
        if unsafe { libc::getpgid(pid) } != pgid {
            return false;
        }
        fs::read(entry.path().join("environ"))
            .is_ok_and(|env| environ_matches(&env, socket, xdisplay))
    })
}

fn parse_pid(name: &OsStr) -> Option<libc::pid_t> {
    std::str::from_utf8(name.as_bytes())
        .ok()
        .and_then(|s| s.parse().ok())
}

// /proc/<pid>/environ is a NUL-separated list of KEY=VALUE entries.
fn environ_matches(
    environ: &[u8],
    socket: &OsStr,
    xdisplay: Option<u32>,
) -> bool {
    let wayland = [b"WAYLAND_DISPLAY=", socket.as_bytes()].concat();
    let display = xdisplay.map(|n| format!("DISPLAY=:{n}").into_bytes());

    environ.split(|b| *b == 0).any(|entry| {
        entry == wayland.as_slice()
            || display.as_deref().is_some_and(|d| entry == d)
    })
}
