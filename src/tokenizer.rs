// tokenizer.rs

use crate::error::{ShellError, ShellPhase};
use anyhow::Result;
use std::{iter::Peekable, str::Chars};

pub enum Token {
    // Words
    Word(String),
    SingleQuoted(String),
    DoubleQuoted(String),

    // Command separation
    Pipe,      // |
    Semicolon, // ;
    Newline,
    And,        // &&
    Or,         // ||
    Background, // &

    // Redirection
    RedirectOut,       // >
    RedirectAppend,    // >>
    RedirectIn,        // <
    RedirectErr,       // 2>
    RedirectErrAndOut, // 2>&1
}

impl Token {
    pub fn is_operator(&self) -> bool {
        use Token::*;
        matches!(self, Pipe | Semicolon | Newline | And | Or | Background)
    }
}

pub struct Tokenizer<'a> {
    line: Peekable<Chars<'a>>,
}

impl<'a> Tokenizer<'a> {
    pub fn tokenize(line: &'a str) -> Result<Vec<Token>> {
        Self {
            line: line.chars().peekable(),
        }
        .run()
    }

    pub fn run(&mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();
        while let Some(&current) = self.line.peek() {
            if current.is_whitespace() {
                self.line.next();
                continue;
            }

            let token = if current == '\'' || current == '"' {
                self.parse_string()?
            } else if self.starts_operator(current) {
                self.parse_operators()?
            } else {
                self.parse_word()?
            };
            tokens.push(token);
        }

        Ok(tokens)
    }

    fn parse_word(&mut self) -> Result<Token> {
        let mut word = String::from(self.line.next().unwrap());
        while let Some(character) = self.line.peek() {
            if !character.is_whitespace() && !Self::is_operator(*character) {
                word.push(self.line.next().unwrap());
            } else {
                break;
            }
        }

        Ok(Token::Word(word))
    }

    fn parse_string(&mut self) -> Result<Token> {
        let quote = self.line.next().unwrap();
        let mut string = String::new();
        while let Some(character) = self.line.next() {
            if character == quote {
                return Ok(if quote == '\'' {
                    Token::SingleQuoted(string)
                } else {
                    Token::DoubleQuoted(string)
                });
            }
            string.push(character);
        }
        self.error(&format!("Found unclosed string starting with ({})", quote))
    }

    fn parse_operators(&mut self) -> Result<Token> {
        match self.line.next().unwrap() {
            '|' => {
                if self.match_next('|') {
                    Ok(Token::Or)
                } else {
                    Ok(Token::Pipe)
                }
            }
            ';' => Ok(Token::Semicolon),
            '\n' => Ok(Token::Newline),
            '&' => {
                if self.match_next('&') {
                    Ok(Token::And)
                } else {
                    Ok(Token::Background)
                }
            }
            '>' => {
                if self.match_next('>') {
                    Ok(Token::RedirectAppend)
                } else {
                    Ok(Token::RedirectOut)
                }
            }
            '<' => Ok(Token::RedirectIn),
            '2' => {
                if self.match_next('>') {
                    if self.match_next('&') {
                        if self.match_next('1') {
                            Ok(Token::RedirectErrAndOut)
                        } else {
                            self.error("Invalid redirection sequence: expected '1' after '2>&'")
                        }
                    } else {
                        Ok(Token::RedirectErr)
                    }
                } else {
                    Ok(Token::Word("2".to_string()))
                }
            }

            other => self.error(&format!("Unexpected character encountered: '{}'", other)),
        }
    }

    fn is_operator(character: char) -> bool {
        matches!(character, '|' | ';' | '&' | '>' | '<' | '\n')
    }

    fn starts_operator(&mut self, ch: char) -> bool {
        if Self::is_operator(ch) {
            return true;
        }
        if ch == '2' && self.line.peek() == Some(&'>') {
            return true;
        }
        false
    }

    fn match_next(&mut self, expected: char) -> bool {
        if self.line.peek() == Some(&expected) {
            self.line.next();
            true
        } else {
            false
        }
    }

    fn error<T>(&self, message: &str) -> Result<T> {
        Err(anyhow::Error::new(ShellError {
            phase: ShellPhase::Tokenizer,
            command: None,
            message: message.into(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize(input: &str) -> Vec<Token> {
        Tokenizer::tokenize(input).unwrap()
    }

    #[test]
    fn test_simple_command() {
        let tokens = tokenize("echo hello");
        assert!(matches!(&tokens[0], Token::Word(w) if w == "echo"));
        assert!(matches!(&tokens[1], Token::Word(w) if w == "hello"));
    }

    #[test]
    fn test_single_quoted() {
        let tokens = tokenize("echo 'hello world'");
        assert!(matches!(&tokens[1], Token::SingleQuoted(s) if s == "hello world"));
    }

    #[test]
    fn test_double_quoted() {
        let tokens = tokenize("echo \"hello world\"");
        assert!(matches!(&tokens[1], Token::DoubleQuoted(s) if s == "hello world"));
    }

    #[test]
    fn test_pipe() {
        let tokens = tokenize("ls | grep foo");
        assert!(matches!(&tokens[1], Token::Pipe));
    }

    #[test]
    fn test_and_or() {
        let tokens = tokenize("foo && bar || baz");
        assert!(matches!(&tokens[1], Token::And));
        assert!(matches!(&tokens[3], Token::Or));
    }

    #[test]
    fn test_redirections() {
        let tokens = tokenize("echo foo > out.txt");
        assert!(matches!(&tokens[2], Token::RedirectOut));
    }

    #[test]
    fn test_redirect_err() {
        let tokens = tokenize("cmd 2> err.txt");
        assert!(matches!(&tokens[1], Token::RedirectErr));
    }

    #[test]
    fn test_redirect_err_and_out() {
        let tokens = tokenize("cmd 2>&1");
        assert!(matches!(&tokens[1], Token::RedirectErrAndOut));
    }

    #[test]
    fn test_unclosed_string() {
        assert!(Tokenizer::tokenize("echo \"unclosed").is_err());
    }

    #[test]
    fn test_word_with_numbers() {
        let tokens = tokenize("echo 123");
        assert!(matches!(&tokens[1], Token::Word(w) if w == "123"));
    }
}
