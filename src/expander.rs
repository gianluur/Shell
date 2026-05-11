//expander.rs

use crate::{
    context::Context,
    error::{ShellError, ShellPhase},
    parser::{Arg, Command, EnvVariable, Redirect, RedirectTarget},
    shell::Shell,
};
use anyhow::{Context as AnyhowContext, Result};
use std::{borrow::Cow, env};

pub fn expand<'a>(
    context: &mut Context,
    command: Command<'a>,
    expanded: &[String],
) -> Result<Command<'static>> {
    match command {
        Command::Simple {
            command,
            args,
            redirects,
            env_vars,
        } => expand_simple_command(context, command, args, redirects, env_vars, expanded),

        Command::Pipeline(left, right) => Ok(Command::Pipeline(
            Box::new(expand(context, *left, expanded)?),
            Box::new(expand(context, *right, expanded)?),
        )),

        Command::And(left, right) => Ok(Command::And(
            Box::new(expand(context, *left, expanded)?),
            Box::new(expand(context, *right, expanded)?),
        )),

        Command::Or(left, right) => Ok(Command::Or(
            Box::new(expand(context, *left, expanded)?),
            Box::new(expand(context, *right, expanded)?),
        )),

        Command::Sequence(left, right) => Ok(Command::Sequence(
            Box::new(expand(context, *left, expanded)?),
            Box::new(expand(context, *right, expanded)?),
        )),

        Command::Background(cmd) => Ok(Command::Background(Box::new(expand(
            context, *cmd, expanded,
        )?))),
    }
}

fn expand_simple_command<'a>(
    context: &mut Context,
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
        let aliased_command = Shell::parse_command(context, &alias, false)
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
                aliased_args.extend(expand_args(context, args)?);
                aliased_redirects.extend(expanded_redirects(context, redirects)?);
                expand(
                    context,
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
                let mut expanded_composed = expand(context, composed_command, &next_expanded)?;
                let extra_args = expand_args(context, args)?;
                let extra_redirects = expanded_redirects(context, redirects)?;
                append_args_to_composed_command(
                    &mut expanded_composed,
                    extra_args,
                    extra_redirects,
                )?;
                Ok(expanded_composed)
            }
        }
    } else {
        Ok(to_owned(context, command, args, redirects, env_vars)?)
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
    }
}

fn expand_string<'a>(context: &mut Context, to_expand: Cow<'a, str>) -> Result<String> {
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
                if chars.peek().is_some() && chars.peek().unwrap().1 == '{' {
                    chars.next();

                    let mut is_ok = false;
                    while let Some(next) = chars.next() {
                        if next.1 == '}' {
                            is_ok = true;
                            break;
                        }
                        variable_name.push(next.1);
                    }

                    if !is_ok {
                        return error("Found unclosed variable expansion bracket '}'");
                    }
                } else {
                    while let Some(&(_, next)) = chars.peek() {
                        if next.is_alphanumeric()
                            || next == '_'
                            || next == '?'
                            || next == '$'
                            || next == '!'
                        {
                            chars.next();
                            variable_name.push(next);
                        } else {
                            break;
                        }
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
                            let expanded_variable = env::var(variable_name).unwrap_or_default();
                            expanded.push_str(&expanded_variable);
                        }
                    };
                }
            }

            _ => expanded.push(character),
        }
    }

    Ok(expanded)
}

fn expand_args(context: &mut Context, args: Vec<Arg>) -> Result<Vec<Arg<'static>>> {
    let mut expanded_args = Vec::new();
    for arg in args {
        let expanded_arg = match arg {
            Arg::Word(s) => Arg::Word(Cow::Owned(expand_string(context, s)?)),
            Arg::DoubleQuoted(s) => Arg::DoubleQuoted(Cow::Owned(expand_string(context, s)?)),
            Arg::SingleQuoted(s) => Arg::SingleQuoted(Cow::Owned(s.into_owned())), // Convert to owned
        };
        expanded_args.push(expanded_arg);
    }

    Ok(expanded_args)
}

fn expanded_redirects(
    context: &mut Context,
    redirects: Vec<Redirect>,
) -> Result<Vec<Redirect<'static>>> {
    let mut expanded_redirects = Vec::new();
    for redirect in redirects {
        let target = match redirect.target {
            RedirectTarget::File(cow) => {
                let expanded_path = expand_string(context, cow)?;
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

pub fn to_owned<'a>(
    context: &mut Context,
    command: Cow<'a, str>,
    args: Vec<Arg>,
    redirects: Vec<Redirect>,
    env_vars: Vec<EnvVariable<'a>>,
) -> Result<Command<'static>> {
    Ok(Command::Simple {
        command: command.into_owned().into(),
        args: expand_args(context, args)?,
        redirects: expanded_redirects(context, redirects)?,
        env_vars: env_vars.into_iter().map(|v| v.into_owned()).collect(),
    })
}

fn error<T>(message: &str) -> Result<T> {
    Err(anyhow::Error::new(ShellError {
        phase: ShellPhase::Expander,
        command: None,
        message: message.into(),
    }))
}
