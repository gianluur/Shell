//executor.rs

use crate::{
    context::Context,
    error::*,
    jobs::{Job, JobState},
    parser::{Command, Redirect, RedirectKind},
    terminal::Terminal,
};

use anyhow::{Context as AnyhowContext, Ok, Result};
use std::{ffi::CString, io, os::fd::RawFd};

pub fn execute(context: &mut Context, command: Command, terminal: &mut Terminal) -> Result<i32> {
    if let Command::Simple {
        command: ref name,
        ref args,
        ..
    } = command
    {
        if let Some(builtin) = context.builtins.get(name) {
            let str_args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            return builtin(&str_args, context, terminal);
        }
    }

    match command {
        Command::Simple { .. } => {
            let pgid = spawn_process(
                context,
                command.clone(),
                libc::STDIN_FILENO,
                libc::STDOUT_FILENO,
                None,
                true,
            )?;

            context
                .jobs
                .wait_foreground(context.gpid, terminal, pgid, command, &[pgid], true)
        }

        Command::And(left, right) => {
            let status = execute(context, *left, terminal)?;
            if status == 0 {
                execute(context, *right, terminal)
            } else {
                Ok(status)
            }
        }

        Command::Or(left, right) => {
            let status = execute(context, *left, terminal)?;
            if status != 0 {
                execute(context, *right, terminal)
            } else {
                Ok(status)
            }
        }

        Command::Sequence(left, right) => {
            execute(context, *left, terminal)?;
            execute(context, *right, terminal)
        }

        Command::Background(command) => {
            let mut fds = [0; 2];
            unsafe {
                if libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) == -1 {
                    return os_error();
                }
            }

            let pipe_read = fds[0];
            let pipe_write = fds[1];

            unsafe {
                let flags = libc::fcntl(pipe_read, libc::F_GETFL);
                libc::fcntl(pipe_read, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }

            if let Command::Simple { .. } = *command {
                let pid = spawn_process(
                    context,
                    *command.clone(),
                    libc::STDIN_FILENO,
                    pipe_write,
                    None,
                    false,
                )?;

                unsafe { libc::close(pipe_write) };

                let job_id = context.jobs.add(Job::new(
                    pid,
                    vec![pid],
                    *command,
                    JobState::Running,
                    Some(pipe_read),
                ));
                terminal.println(&format!("[{}] {}", job_id, pid))?;

                Ok(0)
            } else {
                let (gpid, pids) = spawn_piped(
                    context,
                    *command.clone(),
                    libc::STDIN_FILENO,
                    pipe_write,
                    None,
                    false,
                )?;

                unsafe { libc::close(pipe_write) };

                let job_id = context.jobs.add(Job::new(
                    gpid,
                    pids.clone(),
                    *command.clone(),
                    JobState::Running,
                    Some(pipe_read),
                ));

                terminal.print(&format!("[{}] ", job_id))?;
                for pid in pids {
                    terminal.print(&format!("{} ", pid))?;
                }
                terminal.println("")?;

                Ok(0)
            }
        }

        Command::Pipeline(..) => {
            let (gpid, pids) = spawn_piped(
                context,
                command.clone(),
                libc::STDIN_FILENO,
                libc::STDOUT_FILENO,
                None,
                true,
            )?;

            context
                .jobs
                .wait_foreground(context.gpid, terminal, gpid, command, &pids, true)
        }
    }
}

fn spawn_process(
    context: &mut Context,
    command: Command,
    stdin: RawFd,
    stdout: RawFd,
    pgid: Option<libc::pid_t>,
    is_foreground: bool,
) -> Result<libc::pid_t> {
    match command {
        Command::Simple {
            command,
            args,
            redirects,
        } => {
            let str_args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            let (command, args) = to_cstring(&command, &str_args)?;

            unsafe {
                // We do one final conversion from CString to const char*
                let mut argv: Vec<*const libc::c_char> = args.iter().map(|s| s.as_ptr()).collect();
                argv.push(std::ptr::null());

                let pid = libc::fork();

                if pid == -1 {
                    return os_error();
                }

                if pid == 0 {
                    // ── CHILD ──────────────────────────────────────────────────
                    // Put the child in its own process group and give it the terminal
                    libc::setpgid(0, pgid.unwrap_or(0));

                    // Wire up stdin if it's coming from a pipe
                    if stdin != libc::STDIN_FILENO {
                        libc::dup2(stdin, libc::STDIN_FILENO);
                        libc::close(stdin);
                    }

                    // Wire up stdout if it's going into a pipe
                    if stdout != libc::STDOUT_FILENO {
                        libc::dup2(stdout, libc::STDOUT_FILENO);
                        libc::close(stdout);
                    }

                    // Handle file redirections (>, <, >>, 2>, 2>&1)
                    if let Err(_) = set_stdio(redirects) {
                        libc::_exit(1);
                    }

                    // Reset signals to defaults (shell may have ignored some)
                    context.signals.reset();

                    libc::execvp(command.as_ptr(), argv.as_ptr());

                    // execvp only returns on failure
                    libc::_exit(1);
                } else {
                    // ── PARENT ─────────────────────────────────────────────────

                    // Race condition fix: parent also sets the child's pgid
                    libc::setpgid(pid, pgid.unwrap_or(0));

                    // If it's a foreground process and doesn't belong to a pipeline
                    // give him the terminal
                    if pgid.is_none() && is_foreground {
                        libc::tcsetpgrp(libc::STDIN_FILENO, pid);
                    }

                    // Close the pipe ends we handed to the child — we don't need them
                    if stdin != libc::STDIN_FILENO {
                        libc::close(stdin);
                    }
                    if stdout != libc::STDOUT_FILENO {
                        libc::close(stdout);
                    }

                    Ok(pid)
                }
            }
        }

        _ => unreachable!(),
    }
}

fn spawn_piped(
    context: &mut Context,
    command: Command,
    stdin: RawFd,
    stdout: RawFd,
    pgid: Option<libc::pid_t>,
    is_foreground: bool,
) -> Result<(libc::pid_t, Vec<libc::pid_t>)> {
    match command {
        Command::Simple { .. } => {
            let pid = spawn_process(context, command, stdin, stdout, pgid, is_foreground)?;
            Ok((pid, vec![pid]))
        }

        Command::Pipeline(left, right) => {
            let mut fds = [0; 2];
            unsafe {
                // We pass libc::O_CLOEXEC directly into the creation call
                // this allows the pipe ends to be automatically closed when the process
                // calls exec
                if libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) == -1 {
                    return os_error();
                }
            }

            let pipe_read = fds[0];
            let pipe_write = fds[1];

            // Left side writes into the pipe — it must close the read end it doesn't use
            let (pgid, mut pids) =
                spawn_piped(context, *left, stdin, pipe_write, None, is_foreground)?;

            // Right side reads from the pipe — it must close the write end it doesn't use
            let (_, right_pids) = spawn_piped(
                context,
                *right,
                pipe_read,
                stdout,
                Some(pgid),
                is_foreground,
            )?;

            pids.extend(right_pids);

            // We still close the pipes in the parent because O_CLOEXEC works only
            // when the child calles exec() since the parent never does
            // we still have to perform this clean up
            unsafe {
                libc::close(pipe_read);
                libc::close(pipe_write);
            }

            Ok((pgid, pids))
        }

        _ => unreachable!(),
    }
}

fn set_stdio(redirects: Vec<Redirect>) -> Result<()> {
    for redirect in redirects {
        match redirect.kind {
            RedirectKind::ErrAndOut => {
                // 2>&1 - no file opening, just duplication
                unsafe {
                    if libc::dup2(libc::STDOUT_FILENO, libc::STDERR_FILENO) == -1 {
                        return os_error();
                    }
                }
            }
            _ => {
                let path = match redirect.get_target_path() {
                    Some(path) => CString::new(path.as_str()).with_context(|| {
                        format!("Failed to convert target path to CString {}", path)
                    })?,
                    None => unreachable!(
                        "Target path should be always configured for redirects except ErrAndOut"
                    ),
                };

                let (flags, target_fd) = match redirect.kind {
                    RedirectKind::In => (libc::O_RDONLY, libc::STDIN_FILENO),
                    RedirectKind::Out => (
                        libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                        libc::STDOUT_FILENO,
                    ),
                    RedirectKind::Append => (
                        libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
                        libc::STDOUT_FILENO,
                    ),
                    RedirectKind::Err => (
                        libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                        libc::STDERR_FILENO,
                    ),
                    _ => unreachable!(),
                };

                unsafe {
                    let fd = libc::open(path.as_ptr(), flags, 0o644);
                    if fd == -1 {
                        return os_error();
                    }
                    if libc::dup2(fd, target_fd) == -1 {
                        return os_error();
                    }
                    libc::close(fd);
                }
            }
        }
    }

    Ok(())
}

fn to_cstring(raw_command: &str, raw_args: &Vec<String>) -> Result<(CString, Vec<CString>)> {
    let command = CString::new(raw_command)
        .with_context(|| format!("Failed to convert command '{}' to CString", raw_command))?;

    let mut args = vec![command.clone()];

    for arg in raw_args {
        args.push(
            CString::new(arg.as_str())
                .with_context(|| format!("Failed to convert argument '{}' to CString", arg))?,
        );
    }

    Ok((command, args))
}

fn os_error<T>() -> Result<T> {
    error(&io::Error::last_os_error().to_string())
}

fn error<T>(message: &str) -> Result<T> {
    Err(anyhow::Error::new(ShellError {
        phase: ShellPhase::Executor,
        command: None,
        message: message.into(),
    }))
}
