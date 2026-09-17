//! Run bounded commands and stop their process groups before returning.
//!
//! Pipes stay nonblocking so a noisy child cannot prevent timeout or cancellation.
//! Descendants must retain their inherited process group, or have a separate owner
//! that handles termination. SIGKILL of this supervisor cannot run cleanup.
//!
//! Output distinguishes normal exit, timeout and interruption, and records discarded
//! bytes once capture limits are reached. Requiring success also rejects truncated
//! captures. Explicit completion collects child status and stops remaining group
//! members; Drop provides cleanup during unwind. Cleanup errors remain visible even
//! when the command itself failed.

use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(10);
const GRACE: Duration = Duration::from_secs(2);
const STOP_LIMIT: Duration = Duration::from_secs(5);
static INTERRUPT: AtomicI32 = AtomicI32::new(0);

extern "C" fn interrupted(signal: libc::c_int) {
    INTERRUPT.store(signal, Ordering::Relaxed);
}

/// Install once, before launching work. The handler only records cancellation;
/// normal Rust control flow owns cleanup and its diagnostics.
pub fn install_interrupt_handlers() -> io::Result<()> {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: sigaction accepts the initialized structure and null old-action
        // pointer. The handler has C ABI and only touches a lock-free atomic.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = interrupted as *const () as usize;
            libc::sigemptyset(&mut action.sa_mask);
            if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct Capture {
    pub bytes: Vec<u8>,
    pub omitted: u64,
}

impl Capture {
    fn drain(&mut self, pipe: &mut (impl Read + AsRawFd)) -> io::Result<bool> {
        nonblocking(pipe.as_raw_fd())?;
        let mut buffer = [0; 8192];
        // Bound each turn as well as retained bytes: a continuously writing child
        // must not starve the other pipe or the deadline check.
        for _ in 0..32 {
            match pipe.read(&mut buffer) {
                Ok(0) => return Ok(false),
                Ok(count) => {
                    let keep = count.min(OUTPUT_LIMIT - self.bytes.len());
                    self.bytes.extend_from_slice(&buffer[..keep]);
                    self.omitted += (count - keep) as u64;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }
}

#[derive(Debug, PartialEq)]
pub enum Completion {
    Exited(ExitStatus),
    TimedOut,
    Interrupted(i32),
}

pub struct Output {
    pub completion: Completion,
    pub stdout: Capture,
    pub stderr: Capture,
}

impl Output {
    /// Protocol consumers must also reject truncated output on successful exit.
    pub fn require_success(&self) -> io::Result<()> {
        match self.completion {
            Completion::Exited(status) if status.success() => {}
            ref other => {
                return Err(io::Error::other(format!(
                    "command did not succeed: {other:?}"
                )));
            }
        }
        if self.stdout.omitted != 0 || self.stderr.omitted != 0 {
            return Err(io::Error::other(format!(
                "output truncated: {} stdout bytes and {} stderr bytes omitted",
                self.stdout.omitted, self.stderr.omitted
            )));
        }
        Ok(())
    }
}

struct Process {
    child: Child,
    stdout: Capture,
    stderr: Capture,
    closed: bool,
    grace: Duration,
}

impl Process {
    #[cfg(test)]
    fn spawn(command: &mut Command) -> io::Result<Self> {
        Self::spawn_with_stdout(command, Stdio::piped(), GRACE)
    }

    fn spawn_with_stdout(
        command: &mut Command,
        stdout: Stdio,
        grace: Duration,
    ) -> io::Result<Self> {
        command
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(Stdio::piped());
        command.process_group(0);
        let child = command.spawn()?;
        // Own the child before configuring pipes: configuration can fail too.
        Ok(Self {
            child,
            stdout: Capture::default(),
            stderr: Capture::default(),
            closed: false,
            grace,
        })
    }

    fn drain(&mut self) -> io::Result<bool> {
        let stdout = if let Some(pipe) = self.child.stdout.as_mut() {
            self.stdout.drain(pipe)?
        } else {
            false
        };
        let stderr = self.stderr.drain(self.child.stderr.as_mut().unwrap())?;
        Ok(stdout || stderr)
    }

    fn finish(&mut self) -> io::Result<()> {
        if self.closed {
            return Ok(());
        }
        let mut failure = None;
        let mut record = |result: io::Result<()>| {
            if let Err(error) = result {
                failure.get_or_insert(error);
            }
        };
        // A nested supervisor gets a chance to stop groups it owns. The whole
        // inherited group is then stopped even when its leader already exited.
        match self.child.try_wait() {
            Ok(None) => {
                record(signal_group(self.child.id() as i32, libc::SIGTERM));
                let deadline = Instant::now() + self.grace;
                while Instant::now() < deadline {
                    record(self.drain().map(|_| ()));
                    match self.child.try_wait() {
                        Ok(Some(_)) => {
                            // A wrapper can exit before its supervisor child has
                            // finished cleaning separate groups. Wait while any
                            // member of the inherited group remains.
                            if group_absent(self.child.id() as i32) {
                                break;
                            }
                            std::thread::sleep(POLL);
                        }
                        Ok(None) => std::thread::sleep(POLL),
                        Err(error) => {
                            record(Err(error));
                            break;
                        }
                    }
                }
            }
            Ok(Some(_)) => {}
            Err(error) => record(Err(error)),
        }
        record(signal_group(self.child.id() as i32, libc::SIGKILL));
        let deadline = Instant::now() + STOP_LIMIT;
        loop {
            record(self.drain().map(|_| ()));
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    self.closed = true;
                    break;
                }
                Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
                Ok(None) => {
                    record(Err(io::Error::other("child did not exit after SIGKILL")));
                    break;
                }
                Err(error) => {
                    record(Err(error));
                    break;
                }
            }
        }
        // Drain bytes already buffered after exit. Descendants have been signaled;
        // pipe EOF is not used as proof of ownership release.
        loop {
            match self.drain() {
                Ok(false) => break,
                Ok(true) if Instant::now() < deadline => continue,
                Ok(true) => {
                    record(Err(io::Error::other(
                        "output did not stop after group termination",
                    )));
                    break;
                }
                Err(error) => {
                    record(Err(error));
                    break;
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if !self.closed
            && let Err(error) = self.finish()
        {
            eprintln!("process cleanup failed: {error}");
        }
    }
}

pub fn run(command: &mut Command, timeout: Duration) -> io::Result<Output> {
    run_inner(command, timeout, true, Stdio::piped(), GRACE)
}

/// A checker may own additional process groups. Its outer owner allows more
/// shutdown time than the two-second grace used for the checker's own children.
pub fn run_supervisor(command: &mut Command, timeout: Duration) -> io::Result<Output> {
    run_inner(command, timeout, true, Stdio::piped(), GRACE * 4)
}

/// Cleanup must still execute after cancellation has been recorded.
pub fn run_cleanup(command: &mut Command, timeout: Duration) -> io::Result<Output> {
    run_inner(command, timeout, false, Stdio::piped(), GRACE)
}

pub fn run_to_file(
    command: &mut Command,
    timeout: Duration,
    file: std::fs::File,
) -> io::Result<Output> {
    run_inner(command, timeout, true, Stdio::from(file), GRACE)
}

fn run_inner(
    command: &mut Command,
    timeout: Duration,
    cancellable: bool,
    stdout: Stdio,
    grace: Duration,
) -> io::Result<Output> {
    if timeout.is_zero() {
        return Err(io::Error::other("command timeout must be positive"));
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| io::Error::other("command timeout overflow"))?;
    if cancellable && INTERRUPT.load(Ordering::Relaxed) != 0 {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "supervisor interrupted",
        ));
    }
    let mut process = Process::spawn_with_stdout(command, stdout, grace)?;
    let result = (|| {
        loop {
            process.drain()?;
            let signal = INTERRUPT.load(Ordering::Relaxed);
            if cancellable && signal != 0 {
                return Ok(Completion::Interrupted(signal));
            }
            if let Some(status) = process.child.try_wait()? {
                return Ok(Completion::Exited(status));
            }
            if Instant::now() >= deadline {
                return Ok(Completion::TimedOut);
            }
            std::thread::sleep(POLL);
        }
    })();
    let cleanup = process.finish();
    match (result, cleanup) {
        (Ok(completion), Ok(())) => Ok(Output {
            completion,
            stdout: std::mem::take(&mut process.stdout),
            stderr: std::mem::take(&mut process.stderr),
        }),
        (Err(error), Ok(())) => Err(error),
        (Ok(completion), Err(error)) => Err(io::Error::other(format!(
            "command {completion:?}; cleanup failed: {error}"
        ))),
        (Err(error), Err(cleanup)) => Err(io::Error::other(format!(
            "{error}; cleanup failed: {cleanup}"
        ))),
    }
}

fn nonblocking(fd: i32) -> io::Result<()> {
    // SAFETY: fd is a live pipe; F_GETFL/F_SETFL have the integer ABI used here.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags == -1 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn signal(pid: i32, signal: i32) -> io::Result<()> {
    // SAFETY: kill takes integer identifiers and retains no Rust memory.
    if unsafe { libc::kill(pid, signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

fn signal_group(group: i32, signum: i32) -> io::Result<()> {
    let result = signal(-group, signum);
    #[cfg(target_os = "macos")]
    if let Err(error) = &result
        && error.raw_os_error() == Some(libc::EPERM)
    {
        // Darwin can report EPERM for a group containing only zombies. Do
        // not excuse a real permission failure while any live member remains.
        let mut ps = Command::new("/bin/ps");
        ps.args(["-axo", "pgid=,stat="]);
        let output = run_inner(
            &mut ps,
            Duration::from_secs(5),
            false,
            Stdio::piped(),
            GRACE,
        )?;
        output.require_success()?;
        let listing = std::str::from_utf8(&output.stdout.bytes).map_err(io::Error::other)?;
        for row in listing.lines() {
            let mut fields = row.split_whitespace();
            let id = fields
                .next()
                .ok_or_else(|| io::Error::other("missing process group"))?
                .parse::<i32>()
                .map_err(io::Error::other)?;
            let state = fields
                .next()
                .ok_or_else(|| io::Error::other("missing process state"))?;
            if id == group && !state.starts_with('Z') {
                return result;
            }
        }
        return Ok(());
    }
    result
}

fn group_absent(group: i32) -> bool {
    // SAFETY: signal zero observes existence without changing process state.
    unsafe {
        libc::kill(-group, 0) != 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
