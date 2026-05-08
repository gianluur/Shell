//context.rs

use crate::{builtins::BuiltIns, history::History, jobs::Jobs, signals::SignalHandler};
use anyhow::{Result, anyhow};
use libc::{self};
use std::{env, path::PathBuf};

pub struct Context {
    directory: PathBuf,
    pub name: String,
    pub pid: libc::pid_t,
    pub pgid: libc::pid_t,
    pub builtins: BuiltIns,
    pub jobs: Jobs,
    pub signals: SignalHandler,
    pub last_exit_code: i32,
    pub last_job_pid: Option<libc::pid_t>,
    pub history: History,
}

impl Context {
    pub fn new() -> Result<Context> {
        let mut context = Context {
            name: String::from("RShell"),
            directory: PathBuf::from("/"),
            pgid: Self::setup_pgid()?,
            pid: unsafe { libc::getpid() },
            jobs: Jobs::new(),
            builtins: BuiltIns::new(),
            signals: SignalHandler::new(),
            last_exit_code: 0,
            last_job_pid: None,
            history: History::new()?,
        };
        let home_directory = context.update_cwd();
        unsafe {
            env::set_var("OLDPWD", home_directory);
        }

        Ok(context)
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

    pub fn setup_pgid() -> Result<libc::pid_t> {
        unsafe {
            let gpid = libc::getpid();

            // Make the shell leader of it's own process group
            libc::setpgid(0, 0);

            // Give the shell the terminal
            if libc::tcsetpgrp(libc::STDIN_FILENO, gpid) == -1 {
                return Err(anyhow!("Failed to give the termianl to the shell"));
            }

            Ok(gpid)
        }
    }
}
