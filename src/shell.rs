//shell.rs

use crate::{
    context::Context,
    editor::Editor,
    error::ShellError,
    executor, expander,
    parser::{Command, Parser},
    prompt::Prompt,
    terminal::Terminal,
    tokenizer::Tokenizer,
};
use anyhow::Result;

pub struct Shell {
    terminal: Terminal,
    context: Context,
}

impl Shell {
    pub fn new() -> Self {
        Self {
            terminal: Terminal::new(),
            context: Context::new(),
        }
    }

    pub fn run(&mut self) -> Result<()> {
        let mut prompt = Prompt::new();
        let mut editor = Editor::new();

        self.terminal.clear_screen()?;
        self.terminal.enter_raw_mode()?;

        loop {
            if self.context.signals.child_finished() {
                self.context.jobs.update_table(&mut self.terminal)?;
            }

            self.update_prompt(&mut editor, &mut prompt)?;

            let command = self.parse_command(&mut editor, &mut prompt)?;
            if let Some(command) = command {
                if !self.execute_command(command)? {
                    break;
                }
            }
        }

        self.terminal.exit_raw_mode()?;

        Ok(())
    }

    fn update_prompt(&mut self, editor: &mut Editor, prompt: &mut Prompt) -> Result<()> {
        prompt.update(self.context.update_cwd());

        if let Err(e) = editor.set_prompt(&mut self.terminal) {
            self.terminal.println(&format!("Terminal Error: {:?}", e))?;
        }

        Ok(())
    }

    fn parse_command(
        &mut self,
        editor: &mut Editor,
        prompt: &mut Prompt,
    ) -> Result<Option<Command>> {
        let line = editor.read_line(&mut self.context, &mut self.terminal, prompt)?;
        if line.is_empty() {
            return Ok(None);
        }

        let tokens = Tokenizer::tokenize(&line)?;
        let raw_command = Parser::parse(&tokens)?;

        Ok(Some(expander::expand(&mut self.context, raw_command)?))
    }

    fn execute_command(&mut self, command: Command) -> Result<bool> {
        self.terminal.exit_raw_mode()?;

        let result = executor::execute(&mut self.context, command, &mut self.terminal);

        self.terminal.enter_raw_mode()?;

        if let Err(e) = result {
            if let Some(shell_err) = e.downcast_ref::<ShellError>() {
                if shell_err.is_exit() {
                    return Ok(false);
                }
            }
            self.terminal.println(&format!("{:?}", e))?;
        } else {
            self.context.last_exit_code = result.unwrap();
        }

        Ok(true)
    }
}
