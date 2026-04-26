//parser.rs

use crate::error::{ShellError, ShellPhase};
use crate::tokenizer::Token;
use anyhow::Result;
use std::fmt::{self};
use std::iter::Peekable;

#[derive(Clone, Debug)]
pub enum RedirectTarget {
    File(String),
    FileDescriptor(u8),
}

impl fmt::Display for RedirectTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RedirectTarget::File(path) => write!(f, "{}", path),
            RedirectTarget::FileDescriptor(fd) => write!(f, "&{}", fd),
        }
    }
}

#[derive(Clone, Debug)]
pub enum RedirectKind {
    Out,       // >
    Append,    // >>
    In,        // <
    Err,       // 2>
    ErrAndOut, // 2>&1
}

impl fmt::Display for RedirectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RedirectKind::Out => write!(f, ">"),
            RedirectKind::Append => write!(f, ">>"),
            RedirectKind::In => write!(f, "<"),
            RedirectKind::Err => write!(f, "2>"),
            RedirectKind::ErrAndOut => write!(f, "2>&1"),
        }
    }
}

impl RedirectKind {
    pub fn from_token(token: &Token) -> Option<Self> {
        match token {
            Token::RedirectOut => Some(RedirectKind::Out),
            Token::RedirectAppend => Some(RedirectKind::Append),
            Token::RedirectIn => Some(RedirectKind::In),
            Token::RedirectErr => Some(RedirectKind::Err),
            Token::RedirectErrAndOut => Some(RedirectKind::ErrAndOut),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Redirect {
    pub kind: RedirectKind,
    pub target: RedirectTarget,
}

impl Redirect {
    pub fn get_target_path(&self) -> Option<&String> {
        match &self.target {
            RedirectTarget::File(path) => Some(path),
            _ => None,
        }
    }
}

impl fmt::Display for Redirect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            // ErrAndOut usually doesn't have a target in common shell syntax
            // (e.g., "2>&1" is self-contained)
            RedirectKind::ErrAndOut => write!(f, "2>&1"),
            _ => write!(f, "{}{}", self.kind, self.target),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Arg {
    Word(String),
    SingleQuoted(String),
    DoubleQuoted(String)
}

impl fmt::Display for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Arg::Word(s) | Arg::SingleQuoted(s) | Arg::DoubleQuoted(s) => write!(f, "{}", s)
        }
    }
}

impl TryFrom<&Token> for Arg {
    type Error = anyhow::Error; // Specify that we are using anyhow's error

    fn try_from(value: &Token) -> Result<Self> {
        match value {
            Token::Word(s) => Ok(Self::Word(s.clone())),
            Token::SingleQuoted(s) => Ok(Self::SingleQuoted(s.clone())),
            Token::DoubleQuoted(s) => Ok(Self::DoubleQuoted(s.clone())),
            _ => Parser::error("Failed to cast token to to argument, the only value accepted are 'Word', 'SingleQuoted', 'DoubleQuoted'"),
        }
    }
}

impl Into<String> for Arg {
    fn into(self) -> String {
        match self {
            Arg::Word(s) | Arg::SingleQuoted(s) | Arg::DoubleQuoted(s) => s
        }
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    Simple {
        command: String,
        args: Vec<Arg>,
        redirects: Vec<Redirect>,
    },
    Pipeline(Box<Command>, Box<Command>),
    And(Box<Command>, Box<Command>),
    Or(Box<Command>, Box<Command>),
    Sequence(Box<Command>, Box<Command>),
    Background(Box<Command>),
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Simple { command, args, redirects } => {
                write!(f, "{}", command)?;
                if !args.is_empty() {
                    let str_args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
                    write!(f, " {}", str_args.join(" "))?;
                }
                for redirect in redirects {
                    write!(f, " {}", redirect)?;
                }
                Ok(())
            }
            Command::Pipeline(left, right) => write!(f, "{} | {}", left, right),
            Command::And(left, right) => write!(f, "{} && {}", left, right),
            Command::Or(left, right) => write!(f, "{} || {}", left, right),
            Command::Sequence(left, right) => write!(f, "{}; {}", left, right),
            Command::Background(cmd) => write!(f, "{} &", cmd),
        }
    }
}

pub struct Parser<'a> {
    tokens: Peekable<std::slice::Iter<'a, Token>>,
}

impl<'a> Parser<'a> {
    pub fn parse(tokens: &'a [Token]) -> Result<Command> {
        Self {
            tokens: tokens.iter().peekable(),
        }
        .run()
    }

    pub fn run(&mut self) -> Result<Command> {
        if self.tokens.peek().is_some() {
            self.parse_sequence()
        } else {
            Parser::error("Empty input: no tokens found to parse")
        }
    }

    fn parse_sequence(&mut self) -> Result<Command> {
        let mut left = self.parse_and_or()?;

        while let Some(token) = self.tokens.peek() {
            if matches!(token, Token::Semicolon | Token::Newline) {
                self.tokens.next();

                if self.tokens.peek().is_none() {
                    break;
                }

                let right = self.parse_and_or()?;
                left = Command::Sequence(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }

        Ok(left)
    }

    fn parse_and_or(&mut self) -> Result<Command> {
        let mut left = self.parse_pipeline()?;
        while let Some(token) = self.tokens.peek() {
            if matches!(token, Token::And | Token::Or) {
                let operator = self.tokens.next().unwrap();
                let right = self.parse_pipeline()?;

                if matches!(operator, Token::And) {
                    left = Command::And(Box::new(left), Box::new(right));
                } else {
                    left = Command::Or(Box::new(left), Box::new(right));
                }
            } else {
                break;
            }
        }

        Ok(left)
    }

    fn parse_pipeline(&mut self) -> Result<Command> {
        let mut left = self.parse_command()?;
        while let Some(token) = self.tokens.peek() {
            if matches!(token, Token::Pipe) {
                self.tokens.next();
                let right = self.parse_command()?;
                left = Command::Pipeline(Box::new(left), Box::new(right))
            } else {
                break;
            }
        }

        if matches!(self.tokens.peek(), Some(Token::Background)) {
            self.tokens.next();
            return Ok(Command::Background(Box::new(left)));
        }

        Ok(left)
    }

    fn parse_command(&mut self) -> Result<Command> {
        use Token::*;

        let command = match self.tokens.next() {
            Some(Word(command_name)) => command_name.clone(),
            Some(_) => {
                return Parser::error("Syntax error: expected a command name at the start of the expression");
            }
            None => return Parser::error("Unexpected end of input: expected a command"),
        };

        let mut args = Vec::new();
        let mut redirects = Vec::new();

        while let Some(token) = self.tokens.peek() {
            if token.is_operator() {
                break;
            }

            match token {
                Token::Word(_) | Token::SingleQuoted(_) | Token::DoubleQuoted(_) => {
                    let arg = self.tokens.next().unwrap().try_into()?;
                    args.push(arg);
                }
                RedirectIn | RedirectOut | RedirectAppend | RedirectErr | RedirectErrAndOut => {
                    redirects.push(self.parse_redirect()?);
                }
                _ => break,
            }
        }

        Ok(Command::Simple {
            command,
            args,
            redirects,
        })
    }

    fn parse_redirect(&mut self) -> Result<Redirect> {
        let kind_token = self.tokens.next().unwrap();

        if matches!(kind_token, Token::RedirectErrAndOut) {
            return Ok(Redirect {
                kind: RedirectKind::ErrAndOut,
                target: RedirectTarget::FileDescriptor(1),
            });
        }

        match self.tokens.next() {
            Some(Token::Word(file)) => Ok(Redirect {
                kind: RedirectKind::from_token(kind_token).unwrap(),
                target: RedirectTarget::File(file.clone()),
            }),
            Some(_) => Parser::error("Redirection error: expected a file path, but found an operator or special token"),
            None => Parser::error("Redirection error: expected a file path after the redirect operator, but reached end of input"),
        }
    }

    fn error<T>(message: &str) -> Result<T> {
        Err(anyhow::Error::new(ShellError {
            phase: ShellPhase::Parser,
            command: None,
            message: message.into(),
        }))
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::tokenizer::Tokenizer;

//     fn parse(input: &str) -> Command {
//         let tokens = Tokenizer::tokenize(input).unwrap();
//         Parser::parse(&tokens).unwrap()
//     }

//     #[test]
//     fn test_simple_command() {
//         let cmd = parse("echo hello world");
//         assert!(matches!(cmd, Command::Simple { command, args, .. }
//             if command == "echo" && args == vec!["hello", "world"]));
//     }

//     #[test]
//     fn test_single_quoted_arg() {
//         let cmd = parse("echo 'hello world'");
//         assert!(matches!(cmd, Command::Simple { args, .. }
//             if args == vec!["hello world"]));
//     }

//     #[test]
//     fn test_pipeline() {
//         let cmd = parse("ls | grep foo");
//         assert!(matches!(cmd, Command::Pipeline(_, _)));
//     }

//     #[test]
//     fn test_and() {
//         let cmd = parse("make && make install");
//         assert!(matches!(cmd, Command::And(_, _)));
//     }

//     #[test]
//     fn test_or() {
//         let cmd = parse("cd foo || echo nope");
//         assert!(matches!(cmd, Command::Or(_, _)));
//     }

//     #[test]
//     fn test_sequence() {
//         let cmd = parse("echo hello; echo world");
//         assert!(matches!(cmd, Command::Sequence(_, _)));
//     }

//     #[test]
//     fn test_background() {
//         let cmd = parse("sleep 10 &");
//         assert!(matches!(cmd, Command::Background(_)));
//     }

//     #[test]
//     fn test_redirect_out() {
//         let cmd = parse("echo foo > out.txt");
//         assert!(matches!(cmd, Command::Simple { redirects, .. }
//             if matches!(redirects[0].kind, RedirectKind::Out)));
//     }

//     #[test]
//     fn test_redirect_in() {
//         let cmd = parse("grep foo < in.txt");
//         assert!(matches!(cmd, Command::Simple { redirects, .. }
//             if matches!(redirects[0].kind, RedirectKind::In)));
//     }

//     #[test]
//     fn test_multiple_redirects() {
//         let cmd = parse("cmd < in.txt > out.txt 2> err.txt");
//         assert!(matches!(cmd, Command::Simple { redirects, .. }
//             if redirects.len() == 3));
//     }

//     #[test]
//     fn test_trailing_semicolon() {
//         let cmd = parse("echo hello;");
//         assert!(matches!(cmd, Command::Simple { .. }));
//     }

//     #[test]
//     fn test_pipeline_background() {
//         let cmd = parse("ls | grep foo &");
//         assert!(matches!(cmd, Command::Background(_)));
//     }

//     #[test]
//     fn test_precedence() {
//         // Should parse as: b && (c | d), wrapped in sequence with a
//         let cmd = parse("a ; b && c | d");
//         assert!(matches!(cmd, Command::Sequence(_, right)
//             if matches!(*right, Command::And(_, _))));
//     }

//     #[test]
//     fn test_empty_input() {
//         assert!(Parser::parse(&[]).is_err());
//     }
// }
