//signals.rs

use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicI32, Ordering};

static PIPE_WRITE_END: AtomicI32 = AtomicI32::new(-1);

extern "C" fn sigchld_handler(_: libc::c_int) {
    let fd = PIPE_WRITE_END.load(Ordering::Relaxed);
    if fd != -1 {
        let byte = &1u8 as *const u8 as *const libc::c_void;
        unsafe {
            libc::write(fd, byte, 1);
        }
    }
}

pub struct SignalHandler {
    pub sigchld_fd: RawFd,
}

impl Drop for SignalHandler {
    fn drop(&mut self) {
        unsafe {
            let write_fd = PIPE_WRITE_END.load(Ordering::Relaxed);
            libc::close(self.sigchld_fd);
            libc::close(write_fd);
        }
    }
}

impl SignalHandler {
    pub fn new() -> Self {
        Self::ignore();
        let sigchld_fd = Self::setup_self_pipe_trick();
        Self { sigchld_fd }
    }

    pub fn child_finished(&self) -> bool {
        let mut buffer = [0u8; 64];
        let childs = unsafe {
            libc::read(
                self.sigchld_fd,
                buffer.as_mut_ptr() as *mut libc::c_void,
                64,
            )
        };
        childs > 0
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

    fn setup_self_pipe_trick() -> RawFd {
        unsafe {
            // Create the pipe
            let mut fds = [0i32; 2];
            libc::pipe(fds.as_mut_ptr());

            let [read_end, write_end] = fds;

            // Set the pipe write end to be non blocking
            libc::fcntl(read_end, libc::F_SETFL, libc::O_NONBLOCK);

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

            read_end
        }
    }
}
