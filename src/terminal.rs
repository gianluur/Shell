//terminal.rs

use anyhow::{Context, Result};
use crossterm::{
    cursor::{MoveDown, MoveLeft, MoveRight, MoveTo, MoveUp},
    execute,
    terminal::{Clear, ClearType},
};
use std::{io::{Stdout, Write, stdout}, os::fd::AsRawFd};

pub struct Terminal {
    pub stdout: Stdout,
    pub notifications: Vec<String>,
    is_raw: bool,

}

impl Terminal {
    pub fn new() -> Self {
        Self {
            stdout: stdout(),
            is_raw: false,
            notifications: Vec::new(),
        }
    }

    /// Enter raw mode explicitly
    pub fn enter_raw_mode(&mut self) -> Result<()> {
            if !self.is_raw {
                crossterm::terminal::enable_raw_mode()
                    .context("Failed to enable terminal raw mode")?;

                let fd = self.stdout.as_raw_fd();
                unsafe {
                    let mut termios = std::mem::zeroed();
                    libc::tcgetattr(fd, &mut termios);

                    // Keep output processing (OPOST) and force NL to CR-NL (ONLCR)
                    termios.c_oflag |= libc::OPOST | libc::ONLCR;

                    libc::tcsetattr(fd, libc::TCSANOW, &termios);
                }

                self.is_raw = true;
            }
            Ok(())
        }

    /// Exit raw mode explicitly
    pub fn exit_raw_mode(&mut self) -> Result<()> {
        if self.is_raw {
            crossterm::terminal::disable_raw_mode()
                .context("Failed to disable terminal raw mode")?;
            self.is_raw = false;
        }
        Ok(())
    }

    /// Prints to the screen any output
    pub fn print(&mut self, output: &str) -> Result<()> {
        write!(self.stdout, "{}", output)
            .with_context(|| format!("Failed to write output to terminal: {}", output))?;
        self.stdout
            .flush()
            .context("Failed to flush terminal stdout")?;
        Ok(())
    }

    /// Prints to the screen any output with and goes to a new line
    pub fn println(&mut self, output: &str) -> Result<()> {
        // In raw mode, \r\n is essential for proper carriage return and newline
        write!(self.stdout, "{}\r\n", output)
            .with_context(|| format!("Failed to write line to terminal: {}", output))?;
        self.stdout
            .flush()
            .context("Failed to flush terminal stdout")?;
        Ok(())
    }

    /// Moves to the cursor at the specified column and row
    pub fn move_to(&mut self, column: u16, row: u16) -> Result<()> {
        execute!(self.stdout, MoveTo(column, row))
            .with_context(|| format!("Failed to move cursor to ({}, {})", column, row))?;
        Ok(())
    }

    /// Moves the cursor up
    pub fn move_up(&mut self) -> Result<()> {
        execute!(self.stdout, MoveUp(1)).context("Failed to move cursor up")?;
        Ok(())
    }

    /// Moves the cursor down
    pub fn move_down(&mut self) -> Result<()> {
        execute!(self.stdout, MoveDown(1)).context("Failed to move cursor down")?;
        Ok(())
    }

    /// Moves the cursor right
    pub fn move_right(&mut self) -> Result<()> {
        execute!(self.stdout, MoveRight(1)).context("Failed to move cursor right")?;
        Ok(())
    }

    /// Moves the cursor left
    pub fn move_left(&mut self) -> Result<()> {
        execute!(self.stdout, MoveLeft(1)).context("Failed to move cursor left")?;
        Ok(())
    }

    /// Clears the entire terminal screen
    pub fn clear_screen(&mut self) -> Result<()> {
        execute!(self.stdout, Clear(ClearType::All), MoveTo(0, 0))
            .context("Failed to clear screen and reset cursor position")?;
        Ok(())
    }

    /// Clear the entire line at y height
    pub fn clear_line(&mut self, y: u16) -> Result<()> {
        execute!(self.stdout, MoveTo(0, y), Clear(ClearType::CurrentLine))
            .with_context(|| format!("Failed to clear terminal line at height {}", y))?;
        Ok(())
    }

    /// Retrieves the cursor position
    pub fn cursor_pos(&mut self) -> Result<(u16, u16)> {
        crossterm::cursor::position().context("Failed to retrieve cursor position")
    }
}

/// Important: Drop ensures the user isn't stuck in raw mode if the shell crashes.
impl Drop for Terminal {
    fn drop(&mut self) {
        // We ignore the error in drop because we can't do much with it here
        let _ = self.exit_raw_mode();
    }
}
