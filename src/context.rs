//context.rs

use crate::{
    aliases::Aliases, builtins::BuiltIns, history::History, jobs::Jobs, shell::Shell,
    signals::SignalHandler, terminal::Terminal,
};
use anyhow::{Context as AnyhowContext, Result, anyhow};
use libc::{self};
use std::{env, fs::OpenOptions, io::Read, path::PathBuf};

pub struct Context {
    pub directory: PathBuf,
    pub name: String,
    pub pid: libc::pid_t,
    pub pgid: libc::pid_t,
    pub builtins: BuiltIns,
    pub jobs: Jobs,
    pub signals: SignalHandler,
    pub last_exit_code: i32,
    pub last_job_pid: Option<libc::pid_t>,
    pub history: History,
    pub aliases: Aliases,
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
            signals: SignalHandler::new()?,
            last_exit_code: 0,
            last_job_pid: None,
            history: History::new()?,
            aliases: Aliases::new(),
        };

        Self::setup_home_directory(&mut context);
        Self::exec_config_file(&mut context)?;

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
                return Err(anyhow!("Failed to give the terminal to the shell"));
            }

            Ok(gpid)
        }
    }

    pub fn setup_home_directory(context: &mut Context) {
        let home_directory = context.update_cwd();
        unsafe {
            env::set_var("OLDPWD", home_directory);
        }
    }

    pub fn exec_config_file(context: &mut Context) -> Result<()> {
        let home_dir = env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let path = PathBuf::from(home_dir).join(".rshellrc");

        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)
            .context("Failed to read config file")?;

        let mut content = String::new();
        file.read_to_string(&mut content)
            .context("Failed to read config file")?;

        let mut terminal = Terminal::new();
        for line in content.lines() {
            let command = Shell::parse_command(context, &mut terminal, &line, true)?;
            if !Shell::execute_command(context, &mut terminal, command)?.0 {
                println!(
                    "Exit command was found in rshellrc, it's suggested not to do that,
                    the shell will not shutdown because otherewise you wouldn't be able to open it again"
                );
                break;
            }
        }

        Ok(())
    }
}
