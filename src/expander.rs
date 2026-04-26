//expander.rs

use crate::error::{ShellError, ShellPhase};
use anyhow::Result;
use std::env;

use crate::context::Context;
use crate::parser::{Arg, Command};

pub fn expand(context: &mut Context, command: Command) -> Result<Command> {
    match command {
        Command::Simple {
            command,
            args,
            redirects,
        } => {
            let mut expanded_args: Vec<Arg> = Vec::new();
            for arg in args {
                let expanded_arg = match arg {
                    Arg::Word(s) => Arg::Word(expand_string(context, s)?),
                    Arg::DoubleQuoted(s) => Arg::DoubleQuoted(expand_string(context, s)?),
                    Arg::SingleQuoted(s) => Arg::SingleQuoted(s),
                };
                expanded_args.push(expanded_arg);
            }

            Ok(Command::Simple {
                command: expand_string(context, command)?,
                args: expanded_args,
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

fn expand_string(context: &mut Context, to_expand: String) -> Result<String> {
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
                    if variable_name == "?" {
                        expanded.push_str(&context.last_exit_code.to_string());
                    } else {
                        let expanded_variable = env::var(variable_name).unwrap_or_default();
                        expanded.push_str(&expanded_variable);
                    }
                }
            }

            _ => expanded.push(character),
        }
    }

    Ok(expanded)
}

fn error<T>(message: &str) -> Result<T> {
    Err(anyhow::Error::new(ShellError {
        phase: ShellPhase::Expander, // Updated to Parser phase
        command: None,
        message: message.into(),
    }))
}

// #[cfg(test)]
// mod tests {
//     use super::*;

//     // Helper to simplify tests and handle the Result
//     fn expand(input: &str) -> String {
//         expand_string(input.to_string()).expect("Expansion failed unexpectedly")
//     }

//     #[test]
//     fn test_plain_string() {
//         assert_eq!(expand("hello world"), "hello world");
//     }

//     #[test]
//     fn test_tilde_expansion() {
//         // We handle the case where HOME might not be set in the test environment
//         if let Ok(home) = env::var("HOME") {
//             assert_eq!(expand("~/foo"), format!("{}/foo", home));
//         } else {
//             // If no HOME, POSIX says it stays as ~ based on your match logic
//             assert_eq!(expand("~/foo"), "~/foo");
//         }
//     }

//     #[test]
//     fn test_tilde_not_at_start() {
//         assert_eq!(expand("foo~bar"), "foo~bar");
//     }

//     #[test]
//     fn test_variable_expansions() {
//         // Setting env vars for the duration of this test
//         unsafe {
//             env::set_var("TEST_VAR", "hello");
//         }

//         assert_eq!(expand("$TEST_VAR"), "hello");
//         assert_eq!(expand("hello $TEST_VAR"), "hello hello");
//         assert_eq!(expand("${TEST_VAR}"), "hello");
//         assert_eq!(expand("$TEST_VAR/foo"), "hello/foo");
//     }

//     #[test]
//     fn test_undefined_variable() {
//         // Ensure the var definitely doesn't exist
//         unsafe {
//             env::remove_var("UNDEFINED_VAR_XYZ");
//         }
//         assert_eq!(expand("$UNDEFINED_VAR_XYZ"), "");
//     }

//     #[test]
//     fn test_lone_dollar() {
//         // A lone $ with no alphanumeric chars following should probably stay a $
//         // or follow your specific logic (currently it returns empty)
//         assert_eq!(expand("$"), "");
//     }

//     #[test]
//     fn test_unclosed_bracket_error() {
//         let result = expand_string("${UNCLOSED".to_string());
//         assert!(result.is_err());
//         let err = result.unwrap_err().to_string();
//         assert!(err.contains("unclosed variable expansion bracket"));
//     }
// }
