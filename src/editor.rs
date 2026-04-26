//editor.rs

use anyhow::{Context as AnyhowContext, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

use crate::{context::Context, prompt::Prompt, terminal::Terminal};

struct Buffer {
    data: String,
    index: usize,
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            data: String::new(),
            index: 0,
        }
    }

    /// Inserts data and updates the index by a character (UTF8)
    pub fn insert(&mut self, character: char) {
        self.data.insert(self.index, character);
        self.index += character.len_utf8();
    }

    /// Clears the previous character, returns true if it removed something false otherwise
    pub fn backspace(&mut self) -> bool {
        if self.index == 0 {
            return false;
        }
        // Step back by the previous character's byte length, not just 1
        self.index -= self.prev_char().len_utf8();
        self.data.remove(self.index);
        true
    }

    /// Returns the next word in the buffer
    pub fn next_word(&self) -> usize {
        let remaining = &self.data[self.index..];
        remaining
            .char_indices()
            .skip_while(|(_, c)| !c.is_whitespace()) // Skip current word
            .skip_while(|(_, c)| c.is_whitespace()) // Skip spaces
            .map(|(i, _)| self.index + i)
            .next()
            .unwrap_or(self.data.len()) // If no more words, go to end
    }

    /// Returns the previous word in the buffer
    pub fn prev_word(&self) -> usize {
        let left_portion = &self.data[..self.index];
        left_portion
            .char_indices()
            .rev()
            .skip_while(|(_, c)| c.is_whitespace()) // Skip trailing spaces
            .skip_while(|(_, c)| !c.is_whitespace()) // Skip the word
            .map(|(i, _)| i)
            .next()
            // We usually want the start of the word after the space we found
            .map(|i| {
                self.data[i..self.index]
                    .char_indices()
                    .find(|(_, c)| !c.is_whitespace())
                    .map(|(sub_i, _)| i + sub_i)
                    .unwrap_or(i)
            })
            .unwrap_or(0)
    }

    /// Returns the previous character in the buffer
    fn prev_char(&self) -> char {
        self.data[..self.index].chars().next_back().unwrap()
    }

    /// Returns the next character in the buffer
    fn next_char(&self) -> char {
        self.data[self.index..].chars().next().unwrap()
    }

    /// Get's the buffer lenght
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Clears the buffer efficiently and returns it to shell
    pub fn take(&mut self) -> String {
        self.index = 0;
        std::mem::take(&mut self.data)
    }

    pub fn content(&mut self) -> String {
        self.data.clone()
    }

    pub fn set(&mut self, new_value: &str) {
        self.index = new_value.len();
        self.data = new_value.to_string();
    }
}

pub struct Editor {
    buffer: Buffer,
    row: u16,
    history: Vec<String>,
    history_row: usize,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            buffer: Buffer::new(),
            row: 0,
            history: Vec::new(),
            history_row: 0,
        }
    }

    // Called by Shell before each read_line so the editor knows the current prompt
    pub fn set_prompt(&mut self, terminal: &mut Terminal) -> Result<()> {
        let (_, row) = terminal.cursor_pos()?;
        self.row = row;
        Ok(())
    }

    pub fn read_line(
        &mut self,
        context: &mut Context,
        terminal: &mut Terminal,
        prompt: &Prompt,
    ) -> Result<String> {
        self.redraw(context, terminal, prompt, false)?;

        loop {
            if context.signals.child_finished() {
                self.redraw(context, terminal, prompt, true)?;
            }

            // Prints the current output of any background process
            for line in context.jobs.get_bg_job_stdout()? {
                terminal.print(&line)?;
            }

            let (_, row) = terminal.cursor_pos()?;
            self.row = row;

            // Check for keyboard input with short timeout
            if event::poll(std::time::Duration::from_millis(50))? {
                match event::read().context("Failed to read event")? {
                    Event::Key(KeyEvent {
                        code, modifiers, ..
                    }) => {
                        if modifiers.contains(KeyModifiers::CONTROL) {
                            match code {
                                KeyCode::Char('c') => self.ctrl_c(context, terminal, prompt)?,
                                KeyCode::Char('l') => self.ctrl_l(context, terminal, prompt)?,
                                _ => {}
                            }
                        } else if modifiers.contains(KeyModifiers::ALT) {
                            match code {
                                KeyCode::Left => self.alt_left(context, terminal, prompt)?,
                                KeyCode::Right => self.alt_right(context, terminal, prompt)?,
                                _ => {}
                            }
                        } else {
                            match code {
                                KeyCode::Char(c) => {
                                    self.buffer.insert(c);
                                    self.redraw(context, terminal, prompt, false)?;
                                }
                                KeyCode::Enter => return self.enter(terminal),
                                KeyCode::Backspace => self.backspace(context, terminal, prompt)?,
                                KeyCode::Up => self.up_arrow(context, terminal, prompt)?,
                                KeyCode::Down => self.down_arrow(context, terminal, prompt)?,
                                KeyCode::Left => self.left_arrow(terminal)?,
                                KeyCode::Right => self.right_arrow(terminal)?,
                                KeyCode::Home => self.home_key(terminal, prompt)?,
                                KeyCode::End => self.end_key(terminal, prompt)?,
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn ctrl_c(
        &mut self,
        context: &mut Context,
        terminal: &mut Terminal,
        prompt: &Prompt,
    ) -> Result<()> {
        self.buffer.take();
        terminal.clear_line(self.row)?;
        self.redraw(context, terminal, prompt, false)
    }

    fn ctrl_l(
        &mut self,
        context: &mut Context,
        terminal: &mut Terminal,
        prompt: &Prompt,
    ) -> Result<()> {
        self.row = 0;
        terminal.clear_screen()?;
        terminal.move_to(0, 0)?;
        self.redraw(context, terminal, prompt, false)
    }

    fn alt_left(
        &mut self,
        context: &mut Context,
        terminal: &mut Terminal,
        prompt: &Prompt,
    ) -> Result<()> {
        self.buffer.index = self.buffer.prev_word();
        self.redraw(context, terminal, prompt, false)
    }

    fn alt_right(
        &mut self,
        context: &mut Context,
        terminal: &mut Terminal,
        prompt: &Prompt,
    ) -> Result<()> {
        self.buffer.index = self.buffer.next_word();
        self.redraw(context, terminal, prompt, false)
    }

    fn enter(&mut self, terminal: &mut Terminal) -> Result<String> {
        terminal.println("")?;
        let line = self.buffer.content();
        if !line.is_empty() && self.history.last() != Some(&line) {
            self.history.push(line);
        }
        self.history_row = self.history.len();
        Ok(self.buffer.take())
    }

    fn backspace(
        &mut self,
        context: &mut Context,
        terminal: &mut Terminal,
        prompt: &Prompt,
    ) -> Result<()> {
        if self.buffer.backspace() {
            self.redraw(context, terminal, prompt, false)?;
        }
        Ok(())
    }

    fn up_arrow(
        &mut self,
        context: &mut Context,
        terminal: &mut Terminal,
        prompt: &Prompt,
    ) -> Result<()> {
        if self.history_row > 0 {
            self.history_row -= 1;
            self.buffer.set(&self.history[self.history_row]);
            self.redraw(context, terminal, prompt, false)?;
        }
        Ok(())
    }

    fn down_arrow(
        &mut self,
        context: &mut Context,
        terminal: &mut Terminal,
        prompt: &Prompt,
    ) -> Result<()> {
        if self.history_row < self.history.len() {
            self.history_row += 1;
            let val = if self.history_row == self.history.len() {
                ""
            } else {
                &self.history[self.history_row]
            };
            self.buffer.set(val);
            self.redraw(context, terminal, prompt, false)?;
        }
        Ok(())
    }

    fn left_arrow(&mut self, terminal: &mut Terminal) -> Result<()> {
        if self.buffer.index > 0 {
            self.buffer.index -= self.buffer.prev_char().len_utf8();
            terminal.move_left()?;
        }
        Ok(())
    }

    fn right_arrow(&mut self, terminal: &mut Terminal) -> Result<()> {
        if self.buffer.index < self.buffer.len() {
            self.buffer.index += self.buffer.next_char().len_utf8();
            terminal.move_right()?;
        }
        Ok(())
    }

    fn home_key(&mut self, terminal: &mut Terminal, prompt: &Prompt) -> Result<()> {
        self.buffer.index = 0;
        terminal.move_to(prompt.len() as u16, self.row)
    }

    fn end_key(&mut self, terminal: &mut Terminal, prompt: &Prompt) -> Result<()> {
        self.buffer.index = self.buffer.len();
        terminal.move_to((self.buffer.index + prompt.len()) as u16, self.row)
    }

    fn redraw(
        &mut self,
        context: &mut Context,
        terminal: &mut Terminal,
        prompt: &Prompt,
        child_finished: bool,
    ) -> Result<()> {
        if child_finished {
            self.handle_child_finished(context, terminal)?;
        }

        terminal.clear_line(self.row)?;
        terminal.print(&prompt.message)?;
        terminal.print(&self.buffer.data)?;

        let cursor_col = prompt.len() + self.buffer.index;
        terminal.move_to(cursor_col as u16, self.row)?;

        Ok(())
    }

    fn handle_child_finished(
        &mut self,
        context: &mut Context,
        terminal: &mut Terminal,
    ) -> Result<()> {
        terminal.println("")?;
        context.signals.child_finished();

        context.jobs.update_table(terminal)?;

        let notifications: Vec<String> = terminal.notifications.drain(..).collect();
        for notification in notifications {
            terminal.println(&notification)?;
        }

        let (_, row) = terminal.cursor_pos()?;
        self.row = row;

        Ok(())
    }
}
