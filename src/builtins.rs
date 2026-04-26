// builtins.rs

use crate::{
    context::Context,
    error::{ShellError, ShellPhase},
    jobs::JobState,
    terminal::Terminal,
};
use anyhow::{Context as AnyhowContext, Result};
use std::{collections::HashMap, env, path::PathBuf};

pub type Builtin = fn(&[String], &mut Context, &mut Terminal) -> Result<i32>;

pub struct BuiltIns {
    programs: HashMap<String, Builtin>,
}

impl BuiltIns {
    pub fn new() -> Self {
        let mut programs: HashMap<String, Builtin> = HashMap::new();
        programs.insert("cd".to_string(), Self::cd);
        programs.insert("exit".to_string(), Self::exit);
        programs.insert("jobs".to_string(), Self::jobs);
        programs.insert("fg".to_string(), Self::fg);
        programs.insert("bg".to_string(), Self::bg);

        Self { programs }
    }

    pub fn get(&self, name: &str) -> Option<Builtin> {
        self.programs.get(name).copied()
    }

    pub fn cd(args: &[String], _: &mut Context, _: &mut Terminal) -> Result<i32> {
        let target = if !args.is_empty() {
            PathBuf::from(&args[0])
        } else {
            let home = env::var("HOME");
            if home.is_err() {
                return Self::error("cd", "HOME environment variable not set");
            }
            PathBuf::from(home.unwrap())
        };

        env::set_current_dir(&target)
            .with_context(|| format!("cd: Failed to change directory to '{}'", target.display()))?;

        Ok(0)
    }

    pub fn exit(_args: &[String], _: &mut Context, _: &mut Terminal) -> Result<i32> {
        Err(ShellError::exit())?
    }

    pub fn jobs(_: &[String], context: &mut Context, terminal: &mut Terminal) -> Result<i32> {
        for (_, job) in &context.jobs.table {
            terminal.println(&job.to_string())?;
        }

        Ok(0)
    }

    pub fn fg(args: &[String], context: &mut Context, terminal: &mut Terminal) -> Result<i32> {
        let job_id = Self::job_id_from_args("fg", args, context)?;

        let (pgid, command, pids) = match context.jobs.table.get(&job_id) {
            Some(job) => (job.pgid, job.command.clone(), job.pids.clone()),
            None => {
                return Self::error(
                    "fg",
                    "Unexpected error, job id doesn't match any entry in the job table",
                );
            }
        };

        let exit_code;
        unsafe {
            // We send the SIGCONT signal to all the child procceses in that gpid
            libc::kill(-pgid, libc::SIGCONT);

            if libc::tcsetpgrp(libc::STDIN_FILENO, pgid) == -1 {
                return Self::error("fg", "Failed to give terminal to job");
            }

            let job = context.jobs.table.get_mut(&job_id).unwrap();
            job.state = JobState::Running;

            exit_code = context.jobs.wait_foreground(
                context.gpid,
                terminal,
                pgid,
                command,
                &pids,
                false,
            )?;
        }

        Ok(exit_code)
    }

    pub fn bg(args: &[String], context: &mut Context, terminal: &mut Terminal) -> Result<i32> {
        let job_id = Self::job_id_from_args("bg", args, context)?;
        let job = match context.jobs.table.get_mut(&job_id) {
            Some(job) => job,
            None => {
                return Self::error(
                    "bg",
                    "Unexpected error, job id doesn't match any entry in the job table",
                );
            }
        };
        unsafe {
            libc::kill(-job.pgid, libc::SIGCONT);
        }

        job.state = JobState::Running;
        terminal.println(&format!("[{}] {}", job_id, job.command))?;

        Ok(0)
    }

    fn job_id_from_args(
        command_name: &str,
        args: &[String],
        context: &mut Context,
    ) -> Result<usize> {
        if !args.is_empty() && args.len() != 1 {
            return Self::error(command_name, "Only one argument is expected");
        }

        let job_id: usize;
        if args.len() == 1 {
            let arg = &args[0];
            if !arg.starts_with('%') {
                return Self::error(
                    command_name,
                    "Argument must always be in the format %<job id>",
                );
            }

            let job_id_str = &arg[1..];
            if job_id_str.is_empty() || !job_id_str.chars().all(|c| c.is_ascii_digit()) {
                return Self::error(command_name, "Invalid job ID: must be a number after %");
            }

            job_id = job_id_str.parse::<usize>().unwrap();
        } else {
            let possible_job_id = context
                .jobs
                .table
                .iter()
                .filter(|(_, job)| matches!(job.state, JobState::Stopped))
                .map(|(&id, _)| id)
                .max();

            job_id = match possible_job_id {
                Some(id) => id,
                None => return Self::error(command_name, "No current jobs"),
            };
        }

        Ok(job_id)
    }

    fn error<T>(name: &str, message: &str) -> Result<T> {
        Err(anyhow::Error::new(ShellError {
            phase: ShellPhase::Executor,
            command: Some(name.to_string()),
            message: message.into(),
        }))
    }
}
