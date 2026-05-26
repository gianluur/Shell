// tokenizer.rs

use crate::error::{ShellError, ShellPhase};
use anyhow::Result;

#[derive(Debug)]
pub enum Token<'a> {
    // Words
    Word(&'a str),
    SingleQuoted(&'a str),
    DoubleQuoted(&'a str),

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

    // Parenthesis
    LeftParen,
    RightParen,
}

impl<'a> Token<'a> {
    pub fn is_operator(&self) -> bool {
        use Token::*;
        matches!(self, Pipe | Semicolon | Newline | And | Or | Background)
    }
}

pub struct Tokenizer<'a> {
    line: &'a str,
    cursor: usize,
}

impl<'a> Tokenizer<'a> {
    pub fn tokenize(line: &'a str) -> Result<Vec<Token<'a>>> {
        Self { line, cursor: 0 }.run()
    }

    pub fn run(&mut self) -> Result<Vec<Token<'a>>> {
        let mut tokens = Vec::new();
        while let Some(current) = self.peek() {
            if current.is_whitespace() {
                self.next();
                continue;
            }

            tokens.push(self.get_token(current)?);
        }

        Ok(tokens)
    }

    fn get_token(&mut self, current: char) -> Result<Token<'a>> {
        if current == '\'' || current == '"' {
            Ok(self.parse_string()?)
        } else if self.starts_operator(current) {
            Ok(self.parse_operators()?)
        } else {
            Ok(self.parse_word()?)
        }
    }

    fn parse_word(&mut self) -> Result<Token<'a>> {
        let start = self.cursor;
        while let Some(character) = self.peek() {
            if character.is_whitespace() || Self::is_operator(character) {
                break;
            }

            if character == '\'' || character == '"' {
                self.parse_string()?;
            } else {
                self.next();
                if self.is_subcommand(character) {
                    self.parse_subcommand()?;
                }
            }
        }

        Ok(Token::Word(&self.line[start..self.cursor]))
    }

    fn parse_string(&mut self) -> Result<Token<'a>> {
        let quote = self.next().unwrap();

        let start = self.cursor;
        while let Some(character) = self.next() {
            if character == quote {
                let end = self.cursor - character.len_utf8();
                let content = &self.line[start..end];

                let string_type = if quote == '\'' {
                    Token::SingleQuoted
                } else {
                    Token::DoubleQuoted
                };

                return Ok(string_type(content));
            }
        }
        self.error(&format!("Found unclosed string starting with ({})", quote))
    }

    fn parse_operators(&mut self) -> Result<Token<'a>> {
        match self.next().unwrap() {
            '2' => {
                self.next();
                if self.match_next('&') {
                    if self.match_next('1') {
                        Ok(Token::RedirectErrAndOut)
                    } else {
                        self.error("Expected '1' after '2>&'")
                    }
                } else {
                    Ok(Token::RedirectErr)
                }
            }
            '|' => {
                if self.match_next('|') {
                    Ok(Token::Or)
                } else {
                    Ok(Token::Pipe)
                }
            }
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
            ';' => Ok(Token::Semicolon),
            '\n' => Ok(Token::Newline),
            '<' => Ok(Token::RedirectIn),
            '(' => Ok(Token::LeftParen),
            ')' => Ok(Token::RightParen),
            _ => self.error("Unexpected operator"),
        }
    }

    fn parse_subcommand(&mut self) -> Result<Token<'a>> {
        let start = self.cursor;

        let mut inside_double_quote = false;
        let mut inside_single_quote = false;
        let mut paren_depth = 0;
        while let Some(character) = self.peek() {
            match character {
                '\'' if !inside_double_quote => {
                    inside_single_quote = !inside_single_quote;
                    self.next();
                }

                '"' if !inside_single_quote => {
                    inside_double_quote = !inside_double_quote;
                    self.next();
                }

                '(' if !inside_single_quote && !inside_double_quote => {
                    paren_depth += 1;
                    self.next();
                }

                ')' if !inside_single_quote && !inside_double_quote => {
                    paren_depth -= 1;
                    self.next();
                    if paren_depth == 0 {
                        let end = self.cursor - character.len_utf8(); // exclude the final ')'
                        let content = &self.line[start + 2..end]; // the +2 is for the '$' and '(' character
                        return Ok(Token::Word(content));
                    }
                }

                _ => {
                    self.next();

                    if self.is_subcommand(character) {
                        let _ = self.parse_subcommand()?;
                    }
                }
            };
        }

        return self.error("Mismatched parenthesis in subcommand, missing ')'");
    }

    fn is_subcommand(&self, character: char) -> bool {
        character == '$' && self.peek().is_some() && self.peek().unwrap() == '('
    }

    fn is_operator(character: char) -> bool {
        matches!(character, '|' | ';' | '&' | '>' | '<' | '\n' | '(' | ')')
    }

    fn starts_operator(&self, ch: char) -> bool {
        if Self::is_operator(ch) {
            return true;
        }
        if ch == '2' && self.peek_nth(1) == Some('>') {
            return true;
        }
        false
    }

    fn match_next(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.next();
            true
        } else {
            false
        }
    }

    fn peek_nth(&self, n: usize) -> Option<char> {
        self.line.get(self.cursor..)?.chars().nth(n)
    }

    fn peek(&self) -> Option<char> {
        self.line.get(self.cursor..)?.chars().next()
    }

    fn next(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.cursor += character.len_utf8();
        Some(character)
    }

    fn error<T>(&self, message: &str) -> Result<T> {
        Err(anyhow::Error::new(ShellError {
            phase: ShellPhase::Tokenizer,
            command: None,
            message: message.into(),
        }))
    }
}
