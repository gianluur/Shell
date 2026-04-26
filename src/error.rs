// error.rs

use std::fmt;

#[derive(Debug)]
pub enum ShellPhase {
    Tokenizer,
    Parser,
    Expander,
    Executor,
}

impl fmt::Display for ShellPhase {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let name = match self {
            ShellPhase::Tokenizer => "Tokenizer",
            ShellPhase::Parser => "Parser",
            ShellPhase::Expander => "Expander",
            ShellPhase::Executor => "Executor",
        };
        write!(f, "{}", name)
    }
}

#[derive(Debug)]
pub struct ShellError {
    pub phase: ShellPhase,
    pub command: Option<String>,
    pub message: String,
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.command {
            Some(cmd) => write!(f, "[ERROR | {}] -> {}: {}", self.phase, cmd, self.message),
            None => write!(f, "[ERROR | {}] -> {}", self.phase, self.message),
        }
    }
}

impl ShellError {
    pub fn exit() -> Self {
        Self {
            phase: ShellPhase::Executor,
            command: None,
            message: "QUIT".to_string(),
        }
    }

    pub fn is_exit(&self) -> bool {
        matches!(self.phase, ShellPhase::Executor) && self.message == "QUIT"
    }
}

impl std::error::Error for ShellError {}
