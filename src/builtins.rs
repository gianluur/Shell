// builtins.rs

use crate::{
    context::Context,
    error::{ShellError, ShellPhase},
    jobs::JobState,
    parser::EnvVariable,
    terminal::Terminal,
};
use anyhow::{Context as AnyhowContext, Result};
use std::{collections::HashMap, env, path::PathBuf};

pub type Builtin = fn(&[&str], &mut Context, &mut Terminal) -> Result<i32>;

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
        programs.insert("history".to_string(), Self::history);
        programs.insert("alias".to_string(), Self::alias);
        programs.insert("unalias".to_string(), Self::unalias);
        programs.insert("export".to_string(), Self::export);
        programs.insert("unset".to_string(), Self::unset);

        Self { programs }
    }

    pub fn get(&self, name: &str) -> Option<Builtin> {
        self.programs.get(name).copied()
    }

    pub fn cd(args: &[&str], _: &mut Context, _: &mut Terminal) -> Result<i32> {
        let target = if !args.is_empty() {
            if args[0] == "-" {
                match env::var("OLDPWD") {
                    Ok(old) => PathBuf::from(old),
                    Err(_) => return Self::error("cd", "OLDPWD environment variable isn't set"),
                }
            } else {
                PathBuf::from(&args[0])
            }
        } else {
            match env::var("HOME") {
                Ok(home) => PathBuf::from(home),
                Err(_) => return Self::error("cd", "HOME not set"),
            }
        };

        if let Ok(current) = env::current_dir() {
            unsafe {
                env::set_var("OLDPWD", current);
            }
        }

        env::set_current_dir(&target)
            .with_context(|| format!("cd: Failed to change directory to '{}'", target.display()))?;

        Ok(0)
    }

    pub fn exit(_args: &[&str], _: &mut Context, _: &mut Terminal) -> Result<i32> {
        Err(ShellError::exit())?
    }

    pub fn jobs(_: &[&str], context: &mut Context, terminal: &mut Terminal) -> Result<i32> {
        for (_, job) in &context.jobs.table {
            terminal.println(&job.to_string())?;
        }

        Ok(0)
    }

    pub fn fg(args: &[&str], context: &mut Context, terminal: &mut Terminal) -> Result<i32> {
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
                context.pgid,
                terminal,
                pgid,
                command,
                &pids,
                false,
            )?;
        }

        Ok(exit_code)
    }

    pub fn bg(args: &[&str], context: &mut Context, terminal: &mut Terminal) -> Result<i32> {
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

    fn job_id_from_args(command_name: &str, args: &[&str], context: &mut Context) -> Result<usize> {
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
            let possible_job_id = context.jobs.get_last_job_id();

            job_id = match possible_job_id {
                Some(id) => id,
                None => return Self::error(command_name, "No current jobs"),
            };
        }

        Ok(job_id)
    }

    pub fn history(_: &[&str], context: &mut Context, terminal: &mut Terminal) -> Result<i32> {
        for (n, line) in context.history.current.iter().enumerate() {
            terminal.println(&format!("{} {}", n, line))?;
        }
        Ok(0)
    }

    pub fn alias(args: &[&str], context: &mut Context, terminal: &mut Terminal) -> Result<i32> {
        let (name, mut value) = Self::check_env_var_args("alias", args)?;

        if args.len() == 0 {
            for (name, value) in context.aliases.get_map() {
                terminal.println(&format!("{name}={value}"))?;
            }
            return Ok(0);
        }

        value = EnvVariable::strip_quotes_from_value(value);

        context.aliases.add(name.to_string(), value.to_string());

        Ok(0)
    }

    pub fn unalias(args: &[&str], context: &mut Context, _: &mut Terminal) -> Result<i32> {
        if args.len() != 1 {
            return Self::error("unalias", "Only accepts 1 parameter");
        }

        let name = args[0];
        if context.aliases.get(name).is_none() {
            return Self::error("unalias", &format!("No alias found for name: {name}"));
        }

        context.aliases.remove(name);

        Ok(0)
    }

    pub fn export(args: &[&str], _: &mut Context, _: &mut Terminal) -> Result<i32> {
        let (name, value) = Self::check_env_var_args("export", args)?;

        unsafe {
            env::set_var(name, value);
        }

        Ok(0)
    }

    pub fn unset(args: &[&str], _: &mut Context, _: &mut Terminal) -> Result<i32> {
        if args.len() > 1 {
            return Self::error("unset", "Only either none or 1 parameter");
        }

        unsafe {
            env::remove_var(args[0]);
        }

        Ok(0)
    }

    fn check_env_var_args<'a>(function_name: &str, args: &'a [&str]) -> Result<(&'a str, &'a str)> {
        if args.len() > 1 {
            return Self::error(function_name, "Only either none or 1 parameter");
        }

        let parts: Vec<&str> = args[0].split('=').collect();
        if parts.len() != 2 {
            return Self::error(function_name, "Invalid format, use name='value'");
        }

        let name = parts[0].trim();
        let value = parts[1].trim();

        if name.len() == 0 {
            return Self::error(function_name, "Name can't be empty");
        }

        if value.len() == 0 {
            return Self::error(function_name, "Value can't be empty");
        }

        Ok((name, value))
    }

    fn error<T>(name: &str, message: &str) -> Result<T> {
        Err(anyhow::Error::new(ShellError {
            phase: ShellPhase::Executor,
            command: Some(name.to_string()),
            message: message.into(),
        }))
    }
}
