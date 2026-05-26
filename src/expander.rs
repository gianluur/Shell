//expander.rs

use crate::{
    context::Context,
    error::{ShellError, ShellPhase},
    executor,
    parser::{Arg, Command, EnvVariable, Redirect, RedirectTarget},
    shell::Shell,
    terminal::Terminal,
};
use anyhow::{Context as AnyhowContext, Result};
use std::{
    borrow::Cow,
    env::{self},
    ffi::{CStr, CString},
};

pub fn expand<'a>(
    context: &mut Context,
    terminal: &mut Terminal,
    command: Command<'a>,
    expanded: &[String],
) -> Result<Command<'static>> {
    match command {
        Command::Simple {
            command,
            args,
            redirects,
            env_vars,
        } => expand_simple_command(
            context, terminal, command, args, redirects, env_vars, expanded,
        ),

        Command::Pipeline(left, right) => Ok(Command::Pipeline(
            Box::new(expand(context, terminal, *left, expanded)?),
            Box::new(expand(context, terminal, *right, expanded)?),
        )),

        Command::And(left, right) => Ok(Command::And(
            Box::new(expand(context, terminal, *left, expanded)?),
            Box::new(expand(context, terminal, *right, expanded)?),
        )),

        Command::Or(left, right) => Ok(Command::Or(
            Box::new(expand(context, terminal, *left, expanded)?),
            Box::new(expand(context, terminal, *right, expanded)?),
        )),

        Command::Sequence(left, right) => Ok(Command::Sequence(
            Box::new(expand(context, terminal, *left, expanded)?),
            Box::new(expand(context, terminal, *right, expanded)?),
        )),

        Command::Background(cmd) => Ok(Command::Background(Box::new(expand(
            context, terminal, *cmd, expanded,
        )?))),

        Command::Subshell(cmd) => Ok(Command::Subshell(Box::new(expand(
            context, terminal, *cmd, expanded,
        )?))),
    }
}

fn expand_simple_command<'a>(
    context: &mut Context,
    terminal: &mut Terminal,
    command: Cow<'a, str>,
    args: Vec<Arg>,
    redirects: Vec<Redirect>,
    env_vars: Vec<EnvVariable>,
    expanded: &[String],
) -> Result<Command<'static>> {
    // This function uses recursiona and might seems a bit complicated, but it's actually quite elegant
    // We start by checking if the current command given is an alias, notice that this is a simple command
    // that though, doesn't imply that the mapped alias can't be a composed command
    // Example: the command give is 'Z' and it maps to -> 'a | b'
    let command_ref = command.as_ref().to_string();
    if let Some(alias) = context.aliases.get(&command_ref).cloned()
        && !expanded.contains(&&command_ref)
    {
        // We need to parse the aliased command, because it was written in just string form
        // So we currently have no way to check where are the arguments, what kind of argument they are etc...
        let aliased_command = Shell::parse_command(context, terminal, &alias, false)
            .context(format!("Failed to parse alias: {alias}"))?;

        // We keep an expanded paramenter to this function that allows us to stop recursion by avoiding infinite loops
        // For example if the user does ls='ls --color' it would try to expand ls infinitely, we avoid that with this array
        // that keeps every alias we encoutered, and even if, we find an alias, if it's already present in the array
        // we stop trying to see what it aliases to.

        // We have to make this a vec, because we're actually passing just a slice, this is mainly done because
        // Vec<String> gives so much hassle with the borrow checker, doing it like this avoids all this problem
        // Just imagine that this part is just me adding the new found alias in the slice
        let mut next_expanded = expanded.to_vec();
        next_expanded.push(command_ref);

        // Now we actually perform the checks on the alias to see what kind of command it was
        match aliased_command {
            Command::Simple {
                command,
                args: mut aliased_args,
                redirects: mut aliased_redirects,
                env_vars,
            } => {
                // This is the best case scenario, it's just a simple command so imagine like cdh='cd ~'
                // We can just expand the args, so '~' can become /home/<user>
                // and we call expand, which will come here once again, see that the command isn't alias,
                // so it will not come at all inside this if statement, and go directly in the else case,
                // where it will return the command with everything properly expanded
                aliased_args.extend(expand_args(context, terminal, args)?);
                aliased_redirects.extend(expanded_redirects(context, terminal, redirects)?);
                expand(
                    context,
                    terminal,
                    Command::Simple {
                        command,
                        args: aliased_args,
                        redirects: aliased_redirects,
                        env_vars,
                    },
                    &next_expanded,
                )
            }

            // This is the case which is a bit more troublesome, beacuse we have the aliased command,
            // but the user aliased to something like a pipeline or a sequence. This is a bit of an hassle
            // because we first of all have to expand the all the individual components of the composed command
            // so imagine like a pipeline the first line calls expand, and it does that a bunch of times for all
            // the simple command present, that's why i said at the start it was quite elegant, because it
            // summarized a lot of work into just a single line of code. Now that have the alis fully expanded,
            // we may have recived some external arguments from the user, image this scenario:
            // alias gl='git log --oneline | head'  here's there wasn't much to expand, but, the user called this command like this
            // gl -n $HOME/myfile   What happens now is that we now have to also pass the arguments, to this aliased command,
            // but we also have to expand them, so we call expand_args. Once we expanded the arguments, we need to append them
            // To the last command of the sequence, pipeline or whatever, in this example, it would've been to head.
            // In order to do that we have to traverse the Command AST, and we do that in the append_args_to_composed
            // That function recursively calls itself, only on the right side of the composed commands
            // (which is the last we mentioned before), and once it reaches the simple command contained in the right side,
            // it extends it's arguments with the extra arguments we expanded before.
            // This is also applies to redirects
            composed_command => {
                let mut expanded_composed =
                    expand(context, terminal, composed_command, &next_expanded)?;
                let extra_args = expand_args(context, terminal, args)?;
                let extra_redirects = expanded_redirects(context, terminal, redirects)?;
                append_args_to_composed_command(
                    &mut expanded_composed,
                    extra_args,
                    extra_redirects,
                )?;
                Ok(expanded_composed)
            }
        }
    } else {
        Ok(to_owned(
            context, terminal, command, args, redirects, env_vars,
        )?)
    }
}

fn append_args_to_composed_command(
    command: &mut Command,
    extra_args: Vec<Arg<'static>>,
    extra_redirects: Vec<Redirect<'static>>,
) -> Result<()> {
    match command {
        Command::Simple {
            args, redirects, ..
        } => {
            args.extend(extra_args);
            redirects.extend(extra_redirects);
            Ok(())
        }
        Command::Pipeline(_, right) => {
            append_args_to_composed_command(right, extra_args, extra_redirects)
        }
        Command::And(_, right) => {
            append_args_to_composed_command(right, extra_args, extra_redirects)
        }
        Command::Or(_, right) => {
            append_args_to_composed_command(right, extra_args, extra_redirects)
        }
        Command::Sequence(_, right) => {
            append_args_to_composed_command(right, extra_args, extra_redirects)
        }
        Command::Background(inner) => {
            append_args_to_composed_command(inner, extra_args, extra_redirects)
        }
        Command::Subshell(inner) => {
            append_args_to_composed_command(inner, extra_args, extra_redirects)
        }
    }
}

fn expand_string<'a>(
    context: &mut Context,
    terminal: &mut Terminal,
    to_expand: Cow<'a, str>,
) -> Result<String> {
    if !to_expand.contains(['$', '~']) {
        return Ok(to_expand.to_string());
    }

    let mut expanded = String::new();
    let mut chars = to_expand.char_indices().peekable();

    while let Some((index, character)) = chars.next() {
        match character {
            '~' if index == 0 => {
                // This should be the proper implementation since POSIX
                // doesn't specify the standard for this situation
                match env::var("HOME") {
                    Ok(home) => expanded.push_str(&home),
                    Err(_) => {
                        expanded.push('~');
                    }
                }
            }

            '$' => {
                let mut variable_name = String::new();

                if let Some((_, paren)) = chars.peek() {
                    // This expands variables
                    if *paren != '(' {
                        if let Some((_, next)) = chars.peek() {
                            if *next == '{' {
                                chars.next();

                                let mut is_ok = false;
                                while let Some((_, next)) = chars.next() {
                                    if next == '}' {
                                        is_ok = true;
                                        break;
                                    }
                                    variable_name.push(next);
                                }

                                if !is_ok {
                                    return error(&format!(
                                        "Found unclosed variable expansion bracket '}}'"
                                    ));
                                }
                            }
                        }

                        while let Some(&(_, next)) = chars.peek() {
                            if next.is_alphanumeric() || matches!(next, '_' | '?' | '$' | '!') {
                                chars.next();
                                variable_name.push(next);
                            } else {
                                break;
                            }
                        }

                        if !variable_name.is_empty() {
                            match variable_name.as_str() {
                                "$" => expanded.push_str(&context.pid.to_string()),
                                "0" => expanded.push_str(&context.name.to_string()),
                                "?" => expanded.push_str(&context.last_exit_code.to_string()),
                                "!" => {
                                    if let Some(pid) = context.last_job_pid {
                                        expanded.push_str(&pid.to_string());
                                    }
                                }
                                _ => {
                                    let expanded_variable =
                                        env::var(variable_name).unwrap_or_default();
                                    expanded.push_str(&expanded_variable);
                                }
                            };
                        }
                    }
                    // This is for parsing subcommands
                    else {
                        chars.next();
                        let mut sub_content = String::new();
                        let mut depth = 1;

                        while let Some((_, c)) = chars.next() {
                            match c {
                                '(' => depth += 1,
                                ')' => depth -= 1,
                                _ => {}
                            }
                            if depth == 0 {
                                break;
                            }
                            sub_content.push(c);
                        }

                        let command = Shell::parse_command(context, terminal, &sub_content, true)?;
                        let output = executor::execute_and_get_stdout(context, terminal, command)?;
                        expanded.push_str(&output.trim()); // Trim often needed for stdout
                    }
                }
            }

            _ => expanded.push(character),
        }
    }

    Ok(expanded)
}

fn expand_args(
    context: &mut Context,
    terminal: &mut Terminal,
    args: Vec<Arg>,
) -> Result<Vec<Arg<'static>>> {
    let mut expanded_args = Vec::new();
    for arg in args {
        match arg {
            Arg::Word(s) => {
                // We first expand the variables and then we do globbing
                let expanded_string = expand_string(context, terminal, s)?;
                let matches = glob_word(&expanded_string)?;
                if matches.is_empty() {
                    expanded_args.push(Arg::Word(Cow::Owned(expanded_string)));
                } else {
                    for m in matches {
                        expanded_args.push(Arg::Word(Cow::Owned(m)));
                    }
                }
            }
            Arg::DoubleQuoted(s) => {
                // We expand variable but not do globbing
                let expanded_str = expand_string(context, terminal, s)?;
                expanded_args.push(Arg::DoubleQuoted(Cow::Owned(expanded_str)));
            }
            Arg::SingleQuoted(s) => {
                // Remains as it is
                expanded_args.push(Arg::SingleQuoted(Cow::Owned(s.into_owned())));
            }
        }
    }
    Ok(expanded_args)
}

fn expanded_redirects(
    context: &mut Context,
    terminal: &mut Terminal,
    redirects: Vec<Redirect>,
) -> Result<Vec<Redirect<'static>>> {
    let mut expanded_redirects = Vec::new();
    for redirect in redirects {
        let target = match redirect.target {
            RedirectTarget::File(cow) => {
                let expanded_path = expand_string(context, terminal, cow)?;
                RedirectTarget::File(Cow::Owned(expanded_path))
            }
            RedirectTarget::FileDescriptor(fd) => RedirectTarget::FileDescriptor(fd),
        };

        expanded_redirects.push(Redirect {
            kind: redirect.kind,
            target,
        });
    }

    Ok(expanded_redirects)
}

fn expand_env_vars<'a>(
    context: &mut Context,
    terminal: &mut Terminal,
    env_vars: Vec<EnvVariable<'a>>,
) -> Result<Vec<EnvVariable<'static>>> {
    let mut expanded_env_vars = Vec::new();
    for var in env_vars {
        expanded_env_vars.push(EnvVariable::new(
            Cow::Owned(var.name.into_owned()),
            Cow::Owned(expand_string(context, terminal, var.value)?),
        ));
    }

    Ok(expanded_env_vars)
}

fn glob_word(pattern: &str) -> Result<Vec<String>> {
    if !pattern.contains(['*', '?', '[']) {
        return Ok(Vec::new());
    }

    let pattern_c = CString::new(pattern)?;
    let mut glob_result: libc::glob_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::glob(
            pattern_c.as_ptr(),
            0,    // default behavior
            None, // no custom error function
            &mut glob_result,
        )
    };

    if result == 0 {
        // Success – one or more matches
        let mut matches = Vec::new();
        for i in 0..glob_result.gl_pathc {
            let path_cstr = unsafe { CStr::from_ptr(*glob_result.gl_pathv.offset(i as isize)) };
            matches.push(path_cstr.to_string_lossy().into_owned());
        }
        unsafe { libc::globfree(&mut glob_result) };
        Ok(matches)
    } else if result == libc::GLOB_NOMATCH {
        // No matches – caller will keep the original word
        Ok(Vec::new())
    } else {
        // Other error (e.g., GLOB_NOSPACE, GLOB_ABORTED)
        error(&format!("glob failed for pattern '{}'", pattern))
    }
}

pub fn to_owned<'a>(
    context: &mut Context,
    terminal: &mut Terminal,
    command: Cow<'a, str>,
    args: Vec<Arg>,
    redirects: Vec<Redirect>,
    env_vars: Vec<EnvVariable<'a>>,
) -> Result<Command<'static>> {
    Ok(Command::Simple {
        command: command.into_owned().into(),
        args: expand_args(context, terminal, args)?,
        redirects: expanded_redirects(context, terminal, redirects)?,
        env_vars: expand_env_vars(context, terminal, env_vars)?,
    })
}

fn error<T>(message: &str) -> Result<T> {
    Err(anyhow::Error::new(ShellError {
        phase: ShellPhase::Expander,
        command: None,
        message: message.into(),
    }))
}
