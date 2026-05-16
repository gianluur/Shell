//parser.rs

use crate::error::{ShellError, ShellPhase};
use crate::tokenizer::Token;
use anyhow::{Context, Ok, Result};
use std::ffi::CString;
use std::{
    borrow::Cow,
    fmt::{self},
    iter::Peekable,
};

#[derive(Clone, Debug)]
pub struct EnvVariable<'a> {
    pub name: Cow<'a, str>,
    pub value: Cow<'a, str>,
}

impl<'a> EnvVariable<'a> {
    pub fn new(name: Cow<'a, str>, value: Cow<'a, str>) -> Self {
        Self { name, value }
    }

    pub fn strip_quotes_from_value(value: &str) -> &str {
        if (value.starts_with('\'') && value.ends_with('\''))
            || (value.starts_with('"') && value.ends_with('"'))
        {
            &value[1..value.len() - 1]
        } else {
            value
        }
    }

    pub fn to_cstring(name: &str, value: &str) -> Result<CString> {
        let formatted = format!("{}={}", name, value);
        CString::new(formatted).context(format!(
            "Failed to convert environment variable {}={} to CString",
            name, value
        ))
    }
}

#[derive(Clone, Debug)]
pub enum RedirectTarget<'a> {
    File(Cow<'a, str>),
    FileDescriptor(u8),
}

impl<'a> fmt::Display for RedirectTarget<'a> {
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
pub struct Redirect<'a> {
    pub kind: RedirectKind,
    pub target: RedirectTarget<'a>,
}

impl<'a> Redirect<'a> {
    pub fn get_target_path(&self) -> Option<&str> {
        match &self.target {
            RedirectTarget::File(cow) => Some(cow.as_ref()),
            _ => None,
        }
    }
}

impl<'a> fmt::Display for Redirect<'a> {
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
pub enum Arg<'a> {
    Word(Cow<'a, str>),
    SingleQuoted(Cow<'a, str>),
    DoubleQuoted(Cow<'a, str>),
}

impl<'a> Arg<'a> {
    pub fn as_str(&self) -> &str {
        match self {
            Arg::Word(s) | Arg::SingleQuoted(s) | Arg::DoubleQuoted(s) => s.as_ref(),
        }
    }
}

impl<'a> fmt::Display for Arg<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Arg::Word(s) => write!(f, "{}", s),
            Arg::SingleQuoted(s) => write!(f, "'{}'", s),
            Arg::DoubleQuoted(s) => write!(f, "\"{}\"", s),
        }
    }
}

impl<'a> TryFrom<&'a Token<'a>> for Arg<'a> {
    type Error = anyhow::Error; // Specify that we are using anyhow's error

    fn try_from(value: &'a Token) -> Result<Self> {
        match value {
            Token::Word(s) => Ok(Self::Word(Cow::Borrowed(s))),
            Token::SingleQuoted(s) => Ok(Self::SingleQuoted(Cow::Borrowed(s))),
            Token::DoubleQuoted(s) => Ok(Self::DoubleQuoted(Cow::Borrowed(s))),
            _ => Parser::error(
                "Failed to cast token to to argument, the only value accepted are 'Word', 'SingleQuoted', 'DoubleQuoted'",
            ),
        }
    }
}

impl<'a> Into<String> for Arg<'a> {
    fn into(self) -> String {
        match self {
            Arg::Word(s) | Arg::SingleQuoted(s) | Arg::DoubleQuoted(s) => s.into_owned(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Command<'a> {
    Simple {
        command: Cow<'a, str>,
        args: Vec<Arg<'a>>,
        redirects: Vec<Redirect<'a>>,
        env_vars: Vec<EnvVariable<'a>>,
    },
    Pipeline(Box<Command<'a>>, Box<Command<'a>>),
    And(Box<Command<'a>>, Box<Command<'a>>),
    Or(Box<Command<'a>>, Box<Command<'a>>),
    Sequence(Box<Command<'a>>, Box<Command<'a>>),
    Background(Box<Command<'a>>),
}

impl<'a> Command<'a> {
    pub fn to_string(&self) -> String {
        match self {
            Command::Simple {
                command,
                args,
                redirects,
                env_vars: _,
            } => {
                let mut result = command.to_string();

                for arg in args {
                    result.push_str(&format!(" {}", arg));
                }

                for redirect in redirects {
                    result.push_str(&format!(" {}", redirect));
                }

                result
            }
            Command::Pipeline(left, right) => {
                format!("{} | {}", left.to_string(), right.to_string())
            }
            Command::And(left, right) => {
                format!("{} && {}", left.to_string(), right.to_string())
            }
            Command::Or(left, right) => {
                format!("{} || {}", left.to_string(), right.to_string())
            }
            Command::Sequence(left, right) => {
                format!("{}; {}", left.to_string(), right.to_string())
            }
            Command::Background(cmd) => {
                format!("{} &", cmd.to_string())
            }
        }
    }
}

pub struct Parser<'a> {
    tokens: Peekable<std::slice::Iter<'a, Token<'a>>>,
}

impl<'a> Parser<'a> {
    pub fn parse(tokens: &'a [Token]) -> Result<Command<'a>> {
        Self {
            tokens: tokens.iter().peekable(),
        }
        .run()
    }

    pub fn run(&mut self) -> Result<Command<'a>> {
        if self.tokens.peek().is_some() {
            self.parse_sequence()
        } else {
            Parser::error("Empty input: no tokens found to parse")
        }
    }

    fn parse_sequence(&mut self) -> Result<Command<'a>> {
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

    fn parse_and_or(&mut self) -> Result<Command<'a>> {
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

    fn parse_pipeline(&mut self) -> Result<Command<'a>> {
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

    fn parse_command(&mut self) -> Result<Command<'a>> {
        use Token::*;

        let mut env_vars: Vec<EnvVariable<'a>> = Vec::new();
        while let Some(token) = self.tokens.peek() {
            if let Token::Word(content) = token
                && content.contains('=')
            {
                self.tokens.next();

                let mut parts = content.splitn(2, '=');
                let name = parts.next();
                let value = parts.next();
                match (name, value) {
                    (Some(name), Some(mut value)) => {
                        if name.trim().len() == 0 {
                            return Parser::error(
                                "Syntax error: the name of a env variable can't be empty",
                            );
                        }

                        if value.trim().len() == 0 {
                            return Parser::error(
                                "Syntax error: the value of a env variable can't be empty",
                            );
                        }

                        value = EnvVariable::strip_quotes_from_value(value);

                        env_vars.push(EnvVariable::new(Cow::Borrowed(name), Cow::Borrowed(value)));
                    }
                    _ => {
                        return Parser::error(
                            "Syntax error: an envirorment variable, must be formatted using name=value syntax",
                        );
                    }
                }
            } else {
                break;
            }
        }

        let command = match self.tokens.next() {
            Some(Word(command_name)) => command_name,
            Some(_) => {
                return Parser::error(
                    "Syntax error: expected a command name at the start of the expression",
                );
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
                Word(_) | SingleQuoted(_) | DoubleQuoted(_) => {
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
            command: Cow::Borrowed(command),
            args,
            redirects,
            env_vars,
        })
    }

    fn parse_redirect(&mut self) -> Result<Redirect<'a>> {
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
                target: RedirectTarget::File(Cow::Borrowed(file)),
            }),
            Some(_) => Parser::error(
                "Redirection error: expected a file path, but found an operator or special token",
            ),
            None => Parser::error(
                "Redirection error: expected a file path after the redirect operator, but reached end of input",
            ),
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
