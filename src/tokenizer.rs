// tokenizer.rs

use crate::error::{ShellError, ShellPhase};
use anyhow::Result;

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

    fn parse_word(&mut self) -> Result<Token<'a>> {
        let start = self.cursor;
        while let Some(character) = self.peek() {
            if !character.is_whitespace() && !Self::is_operator(character) {
                self.next();
            } else {
                break;
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
            _ => self.error("Unexpected operator"),
        }
    }

    fn is_operator(character: char) -> bool {
        matches!(character, '|' | ';' | '&' | '>' | '<' | '\n')
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tokenize<'a>(input: &'a str) -> Vec<Token<'a>> {
        Tokenizer::tokenize(input).unwrap()
    }

    #[test]
    fn test_simple_command() {
        let tokens = tokenize("echo hello");
        assert!(matches!(&tokens[0], Token::Word(w) if *w == "echo"));
        assert!(matches!(&tokens[1], Token::Word(w) if *w == "hello"));
    }

    #[test]
    fn test_single_quoted() {
        let tokens = tokenize("echo 'hello world'");
        assert!(matches!(&tokens[1], Token::SingleQuoted(s) if *s == "hello world"));
    }

    #[test]
    fn test_double_quoted() {
        let tokens = tokenize("echo \"hello world\"");
        assert!(matches!(&tokens[1], Token::DoubleQuoted(s) if *s == "hello world"));
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
        assert!(matches!(&tokens[1], Token::Word(w) if *w == "123"));
    }
}
