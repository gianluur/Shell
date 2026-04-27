//context.rs

use crate::{builtins::BuiltIns, jobs::Jobs, signals::SignalHandler};

use libc::{self};
use std::{env, path::PathBuf};

pub struct Context {
    directory: PathBuf,
    pub name: String,
    pub pid: libc::pid_t,
    pub gpid: libc::pid_t,
    pub builtins: BuiltIns,
    pub jobs: Jobs,
    pub signals: SignalHandler,
    pub last_exit_code: i32,
    pub last_job_pid: Option<libc::pid_t>,
}

impl Context {
    pub fn new() -> Self {
        let mut context = Context {
            name: String::from("RShell"),
            directory: PathBuf::from("/"),
            gpid: Self::setup_pgid(),
            pid: unsafe { libc::getpid() },
            jobs: Jobs::new(),
            builtins: BuiltIns::new(),
            signals: SignalHandler::new(),
            last_exit_code: 0,
            last_job_pid: None,
        };
        let home_directory = context.update_cwd();
        unsafe {
            env::set_var("OLDPWD", home_directory);
        }

        context
    }

    pub fn update_cwd(&mut self) -> &PathBuf {
        if let Ok(cwd) = env::current_dir() {
            self.directory = cwd;
        } else {
            self.directory = env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/"));
        }
        &self.directory
    }

    pub fn setup_pgid() -> libc::pid_t {
        unsafe {
            let gpid = libc::getpid();

            // Make the shell leader of it's own process group
            libc::setpgid(0, 0);

            // Give the shell the terminal
            libc::tcsetpgrp(libc::STDIN_FILENO, gpid);

            gpid
        }
    }
}
