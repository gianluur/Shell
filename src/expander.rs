//expander.rs

use crate::error::{ShellError, ShellPhase};
use anyhow::Result;
use std::borrow::Cow;
use std::env;

use crate::context::Context;
use crate::parser::{Arg, Command, Redirect, RedirectTarget};

pub fn expand<'a>(context: &mut Context, command: Command<'a>) -> Result<Command<'static>> {
    match command {
        Command::Simple {
            command,
            args,
            redirects,
        } => {
            let command = Cow::Owned(expand_string(context, command)?);
            let args = expand_args(context, args)?;
            let redirects = expanded_redirects(context, redirects)?;

            Ok(Command::Simple {
                command,
                args,
                redirects,
            })
        }
        Command::Pipeline(left, right) => Ok(Command::Pipeline(
            Box::new(expand(context, *left)?),
            Box::new(expand(context, *right)?),
        )),
        Command::And(left, right) => Ok(Command::And(
            Box::new(expand(context, *left)?),
            Box::new(expand(context, *right)?),
        )),
        Command::Or(left, right) => Ok(Command::Or(
            Box::new(expand(context, *left)?),
            Box::new(expand(context, *right)?),
        )),
        Command::Sequence(left, right) => Ok(Command::Sequence(
            Box::new(expand(context, *left)?),
            Box::new(expand(context, *right)?),
        )),
        Command::Background(cmd) => Ok(Command::Background(Box::new(expand(context, *cmd)?))),
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
                        if next.is_alphanumeric() || next == '_' || next == '?' {
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

fn error<T>(message: &str) -> Result<T> {
    Err(anyhow::Error::new(ShellError {
        phase: ShellPhase::Expander,
        command: None,
        message: message.into(),
    }))
}
