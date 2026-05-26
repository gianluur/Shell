//signals.rs

use crate::error::{ShellError, ShellPhase};
use anyhow::Result;
use std::{
    io,
    os::unix::io::RawFd,
    sync::atomic::{AtomicI32, Ordering},
};

static PIPE_WRITE_END: AtomicI32 = AtomicI32::new(-1);
static SIGNAL_BYTE: u8 = 1;

extern "C" fn sigchld_handler(_: libc::c_int) {
    let fd = PIPE_WRITE_END.load(Ordering::Relaxed);
    if fd != -1 {
        let byte = &SIGNAL_BYTE as *const u8 as *const libc::c_void;
        unsafe {
            libc::write(fd, byte, 1);
        }
    }
}

#[derive(Clone)]
pub struct SignalHandler {
    pub sigchld_fd: RawFd,
}

impl Drop for SignalHandler {
    fn drop(&mut self) {
        unsafe {
            // Reset signal handler first to prevent races
            let mut signal_action: libc::sigaction = std::mem::zeroed();
            signal_action.sa_sigaction = libc::SIG_DFL;
            libc::sigaction(libc::SIGCHLD, &signal_action, std::ptr::null_mut());

            // Clear out the atomic so the handler (if running) sees -1
            let write_fd = PIPE_WRITE_END.swap(-1, Ordering::Relaxed);

            // Now safely close the file descriptors
            if write_fd != -1 {
                libc::close(write_fd);
            }
            libc::close(self.sigchld_fd);
        }
    }
}

impl SignalHandler {
    pub fn new() -> Result<Self> {
        Self::ignore();
        let sigchld_fd = Self::setup_self_pipe_trick()?;
        Ok(Self { sigchld_fd })
    }

    pub fn dummy() -> Self {
        Self { sigchld_fd: -1 }
    }

    pub fn drain_child_pipe(&self) -> bool {
        let mut buffer = [0u8; 64];
        let bytes_read = unsafe {
            libc::read(
                self.sigchld_fd,
                buffer.as_mut_ptr() as *mut libc::c_void,
                64,
            )
        };
        bytes_read > 0
    }

    pub fn ignore() {
        unsafe {
            // Make it ignore SIGTTOU, SIGTTIN, SIGTSTP so it properly
            // access terminal even where there are background tasks
            libc::signal(libc::SIGTTOU, libc::SIG_IGN);
            libc::signal(libc::SIGTSTP, libc::SIG_IGN);
            libc::signal(libc::SIGTTIN, libc::SIG_IGN);

            // Also ignore SIGINT, SIGQUIT so CTRL + C doesn't kill the shell
            libc::signal(libc::SIGINT, libc::SIG_IGN);
            libc::signal(libc::SIGQUIT, libc::SIG_IGN);
        }
    }

    pub fn reset(&self) {
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGTSTP, libc::SIG_DFL);
            libc::signal(libc::SIGTTOU, libc::SIG_DFL);
            libc::signal(libc::SIGTTIN, libc::SIG_DFL);
        }
    }

    fn setup_self_pipe_trick() -> Result<RawFd> {
        unsafe {
            // Create the pipe and make it non blocking
            let mut fds = [0i32; 2];
            if libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK) == -1 {
                return Self::os_error();
            }

            let [read_end, write_end] = fds;

            // Store the value in the static variable to be passed down to the
            // signal handler C function callback
            PIPE_WRITE_END.store(write_end, Ordering::Relaxed);

            // Setup the signal action with the sigchld hanlder we created
            let mut signal_action: libc::sigaction = std::mem::zeroed();

            // We create a pointer to the function and we cast it to the sighandler_t type
            signal_action.sa_sigaction = sigchld_handler as *const () as libc::sighandler_t;

            // Setup the signal mask (the behavior basically)
            // SA_RESTART: makes system calls interrupted by signals automatically restart.
            // SA_NOCLD_STOP: stops SIGCHLD from arriving when child processes stop. You only get it when they terminate.
            libc::sigemptyset(&mut signal_action.sa_mask);
            signal_action.sa_flags = libc::SA_RESTART | libc::SA_NOCLDSTOP;

            // Create the signal handler itself with everything we did before
            libc::sigaction(libc::SIGCHLD, &signal_action, std::ptr::null_mut());

            Ok(read_end)
        }
    }

    fn os_error<T>() -> Result<T> {
        Self::error(&io::Error::last_os_error().to_string())
    }

    fn error<T>(message: &str) -> Result<T> {
        Err(anyhow::Error::new(ShellError {
            phase: ShellPhase::SignalHandler,
            command: None,
            message: message.into(),
        }))
    }
}
