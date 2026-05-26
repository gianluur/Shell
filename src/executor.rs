//executor.rs

use crate::{
    context::Context,
    error::*,
    jobs::{Job, JobState, Jobs},
    parser::{Command, EnvVariable, Redirect, RedirectKind},
    terminal::Terminal,
};
use anyhow::{Context as AnyhowContext, Ok, Result};
use std::{collections::HashMap, env, ffi::CString, io, os::fd::RawFd};

pub fn execute(
    context: &mut Context,
    terminal: &mut Terminal,
    command: Command<'static>,
    stdout_fd: Option<RawFd>, // if this parameter here is present it means that we're calling this from a subcommand
) -> Result<(i32, libc::pid_t)> {
    if let Command::Simple {
        command: ref name,
        ref args,
        ..
    } = command
    {
        if let Some(builtin) = context.builtins.get(name) {
            let str_args: Vec<&str> = args.iter().map(|a| a.as_str()).collect();
            return Ok((builtin(&str_args, context, terminal)?, 0 as libc::pid_t));
        }
    }

    let stdout = stdout_fd.unwrap_or(libc::STDOUT_FILENO);
    let command_str = command.to_string();
    match command {
        Command::Simple { .. } => {
            let pgid = spawn_process(context, command, libc::STDIN_FILENO, stdout, None, true)?;

            if stdout_fd.is_none() {
                Ok((
                    context.jobs.wait_foreground(
                        context.pgid,
                        terminal,
                        pgid,
                        command_str,
                        &[pgid],
                        true,
                        false,
                    )?,
                    pgid,
                ))
            } else {
                Ok((0, pgid))
            }
        }

        Command::And(left, right) => {
            let status = execute(context, terminal, *left, stdout_fd)?;
            if status.0 == 0 {
                execute(context, terminal, *right, stdout_fd)
            } else {
                Ok(status)
            }
        }

        Command::Or(left, right) => {
            let status = execute(context, terminal, *left, stdout_fd)?;
            if status.0 != 0 {
                execute(context, terminal, *right, stdout_fd)
            } else {
                Ok(status)
            }
        }

        Command::Sequence(left, right) => {
            execute(context, terminal, *left, stdout_fd)?;
            execute(context, terminal, *right, stdout_fd)
        }

        Command::Background(command) => {
            if stdout != libc::STDOUT_FILENO {
                error("You cannot use a background command as a subcommand")?;
            }

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
                    *command,
                    libc::STDIN_FILENO,
                    pipe_write,
                    None,
                    false,
                )?;

                unsafe { libc::close(pipe_write) };

                let job_id = context.jobs.add(Job::new(
                    pid,
                    vec![pid],
                    command_str,
                    JobState::Running,
                    Some(pipe_read),
                ));
                context.last_job_pid = Some(pid);

                terminal.println(&format!("[{}] {}", job_id, pid))?;

                Ok((0, pid))
            } else {
                let (gpid, pids) = spawn_piped(
                    context,
                    *command,
                    libc::STDIN_FILENO,
                    pipe_write,
                    None,
                    false,
                )?;

                unsafe { libc::close(pipe_write) };

                context.last_job_pid = Some(*pids.last().unwrap());

                let job_id = context.jobs.add(Job::new(
                    gpid,
                    pids.clone(),
                    command_str,
                    JobState::Running,
                    Some(pipe_read),
                ));

                terminal.print(&format!("[{}] ", job_id))?;
                for pid in pids {
                    terminal.print(&format!("{} ", pid))?;
                }
                terminal.println("")?;

                Ok((0, gpid))
            }
        }

        Command::Pipeline(..) => {
            let (gpid, pids) =
                spawn_piped(context, command, libc::STDIN_FILENO, stdout, None, true)?;

            if stdout_fd.is_none() {
                Ok((
                    context.jobs.wait_foreground(
                        context.pgid,
                        terminal,
                        gpid,
                        command_str,
                        &pids,
                        true,
                        false,
                    )?,
                    gpid,
                ))
            } else {
                Ok((0, gpid))
            }
        }

        Command::Subshell(command) => {
            let pid = unsafe { libc::fork() };
            if pid == -1 {
                return os_error();
            }

            if pid == 0 {
                // ── CHILD ────────────────────────────────────────────────

                unsafe { libc::setpgid(0, 0) };

                let child_pid = unsafe { libc::getpid() };
                let mut child_context = context.clone().duplicate(child_pid)?;
                context.signals.reset();

                execute(&mut child_context, terminal, *command, None)?;

                unsafe { libc::_exit(0) };
            }

            // ── PARENT ────────────────────────────────────────────────────
            unsafe {
                libc::setpgid(pid, 0);

                if libc::tcsetpgrp(libc::STDIN_FILENO, pid) == -1 {
                    return os_error();
                }
            }

            let mut status = 0;
            unsafe {
                libc::waitpid(pid, &mut status, libc::WNOHANG);
                libc::tcsetpgrp(libc::STDIN_FILENO, context.pgid);
            }

            let exit_code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                1
            };

            Ok((exit_code, pid))
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
            env_vars,
        } => {
            let str_args: Vec<&str> = args.iter().map(|a| a.as_str()).collect();
            let (command, args) = to_cstring(&command, &str_args)?;

            let mut env_map = HashMap::new();
            for var in env::vars_os() {
                env_map.insert(
                    var.0.to_string_lossy().to_string(),
                    var.1.to_string_lossy().to_string(),
                );
            }

            for var in env_vars {
                env_map.insert(
                    var.name.as_ref().to_string(),
                    var.value.as_ref().to_string(),
                );
            }

            let env_vec = env_map
                .iter()
                .map(|(name, value)| EnvVariable::to_cstring(name, value))
                .collect::<Result<Vec<CString>>>()?;

            unsafe {
                // We do one final conversion from CString to const char*
                let mut argv: Vec<*const libc::c_char> = args.iter().map(|s| s.as_ptr()).collect();
                argv.push(std::ptr::null());

                let mut envp: Vec<*const libc::c_char> =
                    env_vec.iter().map(|v| v.as_ptr()).collect();
                envp.push(std::ptr::null());

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

                    libc::execvpe(command.as_ptr(), argv.as_ptr(), envp.as_ptr());

                    // message to the parent the command was not found
                    let _ = libc::write(
                        libc::STDERR_FILENO,
                        b"Command not found\n".as_ptr() as *const _,
                        19,
                    );
                    // execvp only returns on failure
                    libc::_exit(1);
                } else {
                    // ── PARENT ─────────────────────────────────────────────────

                    // Race condition fix: parent also sets the child's pgid
                    libc::setpgid(pid, pgid.unwrap_or(0));

                    // If it's a foreground process and doesn't belong to a pipeline
                    // give him the terminal
                    if pgid.is_none() && is_foreground {
                        if libc::tcsetpgrp(libc::STDIN_FILENO, pid) == -1 {
                            return os_error();
                        }
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

pub fn execute_and_get_stdout(
    context: &mut Context,
    terminal: &mut Terminal,
    command: Command<'static>,
) -> Result<String> {
    let mut pipe_fds = [0; 2];
    unsafe {
        if libc::pipe2(pipe_fds.as_mut_ptr(), libc::O_CLOEXEC) == -1 {
            return os_error();
        }
    }

    let (read_end, write_end) = (pipe_fds[0], pipe_fds[1]);
    let pgid = execute(context, terminal, command, Some(write_end))?.1;

    unsafe {
        libc::close(write_end);
    }

    let mut output = String::with_capacity(4096);
    let mut status = 0;
    loop {
        let ret = unsafe { libc::waitpid(-pgid, &mut status, libc::WNOHANG | libc::WUNTRACED) };

        let finished = if ret == -1 {
            true
        } else if ret > 0 {
            libc::WIFEXITED(status) || libc::WIFSIGNALED(status)
        } else {
            false
        };
        output.push_str(&Jobs::job_stdout_from_fd(read_end)?);

        if finished {
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    unsafe {
        libc::close(read_end);
        if libc::tcsetpgrp(libc::STDIN_FILENO, context.pgid) == -1 {
            return os_error();
        }
    }

    if output.ends_with('\n') {
        output.pop();
        if output.ends_with('\r') {
            output.pop();
        }
    }

    Ok(output)
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
                    Some(path) => CString::new(path).with_context(|| {
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

fn to_cstring(raw_command: &str, raw_args: &[&str]) -> Result<(CString, Vec<CString>)> {
    let command = CString::new(raw_command)
        .with_context(|| format!("Failed to convert command '{}' to CString", raw_command))?;

    let mut args = vec![command.clone()];

    for arg in raw_args {
        args.push(
            CString::new(*arg)
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
