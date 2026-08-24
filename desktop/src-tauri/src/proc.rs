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
//!
//! Windows children are additionally [`adopt`]ed into one kill-on-close Job
//! Object whose only handle lives in this process: if the app dies without
//! running its exit handlers (Ctrl+C on the dev harness, a crash), the OS
//! closes the handle and reaps every adopted child and their descendants.
//! Without it, an aborted dev session leaves a nurb dev tree running from
//! target\debug\uv.exe, which then blocks the next build as a locked file.

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

/// Tie a just-spawned child's fate to this process. On Windows the child (and
/// every process it goes on to create) joins the app's kill-on-close job, so
/// no exit path can orphan it. A no-op on Unix, where app exit runs the
/// shutdown handlers and orphans from a hard kill are the platform norm.
pub fn adopt(pid: u32) {
    #[cfg(windows)]
    job::adopt(pid);
    #[cfg(not(windows))]
    let _ = pid;
}

#[cfg(windows)]
mod job {
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

    struct Job(HANDLE);
    // The handle is only ever passed to Win32 calls, which is thread-safe.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    /// One kill-on-close job for the app's lifetime. Deliberately never
    /// closed: the OS closing it at process death IS the cleanup.
    fn shared() -> HANDLE {
        static JOB: OnceLock<Job> = OnceLock::new();
        JOB.get_or_init(|| Job(kill_on_close_job())).0
    }

    pub(super) fn kill_on_close_job() -> HANDLE {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if !job.is_null() {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const std::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
            }
            job
        }
    }

    pub(super) fn adopt(pid: u32) {
        adopt_into(shared(), pid);
    }

    /// Best-effort: a child that died before adoption, or one this process
    /// may not open, is left to the taskkill path.
    pub(super) fn adopt_into(job: HANDLE, pid: u32) {
        if job.is_null() {
            return;
        }
        unsafe {
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if !process.is_null() {
                AssignProcessToJobObject(job, process);
                CloseHandle(process);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    /// The orphan guarantee: a child adopted into a kill-on-close job dies
    /// when the job's last handle closes, which is what happens to the app's
    /// shared job when the process is killed without running its handlers.
    #[cfg(windows)]
    #[test]
    fn closing_the_job_reaps_an_adopted_child() {
        use windows_sys::Win32::Foundation::CloseHandle;
        let job = job::kill_on_close_job();
        assert!(!job.is_null());
        let mut command = Command::new("cmd");
        command
            .args(["/C", "ping -n 60 127.0.0.1 >NUL"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        setup(&mut command);
        let mut child = command.spawn().unwrap();
        job::adopt_into(job, child.id());

        unsafe { CloseHandle(job) };
        let started = Instant::now();
        loop {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "child survived the job closing"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

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
