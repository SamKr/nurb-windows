//! Process-tree lifecycle, cross-platform. The app spawns children that fan
//! out into their own trees (uv -> python, node -> agent CLI), and every one
//! of them must die with the app or with its owner. On Unix that is a process
//! group: spawn with `process_group(0)` and signal the group. Windows has no
//! process groups worth the name, so the tree is taken down by pid with
//! `taskkill /T`, which walks parent links; children are also spawned with
//! CREATE_NO_WINDOW so console hosts never flash over the app.
//!
//! The id handed around is the child's own pid. On Unix `process_group(0)`
//! makes the group id equal to it, so one integer serves both worlds.

use std::process::Command;

/// Prepare a child so [`kill_tree`] can take its whole tree down later, and
/// (on Windows) so it never opens a console window.
pub fn setup(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Ask the tree rooted at `pid` to stop. Unix sends SIGTERM to the group;
/// Windows has no polite signal a console child would hear from a windowless
/// parent, so this terminates the tree outright, same as [`kill_tree_force`].
pub fn kill_tree(pid: i32) {
    #[cfg(unix)]
    unsafe {
        libc::killpg(pid, libc::SIGTERM);
    }
    #[cfg(windows)]
    taskkill(pid);
}

/// Take the tree down without asking. SIGKILL on Unix; on Windows the same
/// hard termination [`kill_tree`] already is.
pub fn kill_tree_force(pid: i32) {
    #[cfg(unix)]
    unsafe {
        libc::killpg(pid, libc::SIGKILL);
    }
    #[cfg(windows)]
    taskkill(pid);
}

/// `taskkill /T` resolves the tree via parent-pid links at kill time, so it
/// runs before the root is reaped (the callers all wait the child afterwards).
/// Output is captured, not shown; a tree already gone exits nonzero and that
/// is fine.
#[cfg(windows)]
fn taskkill(pid: i32) {
    if pid <= 0 {
        return;
    }
    let mut command = Command::new("taskkill");
    command.args(["/T", "/F", "/PID", &pid.to_string()]);
    setup(&mut command);
    let _ = command.output();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    /// The property every caller relies on: after kill_tree, the child (and
    /// so its tree) is reapable promptly.
    #[test]
    fn a_killed_tree_is_gone() {
        let mut command = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "ping -n 60 127.0.0.1 >NUL"]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", "sleep 60"]);
            c
        };
        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        setup(&mut command);
        let mut child = command.spawn().unwrap();
        let pid = child.id() as i32;

        let started = Instant::now();
        kill_tree(pid);
        #[cfg(unix)]
        kill_tree_force(pid);
        loop {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(5), "child survived kill_tree");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
