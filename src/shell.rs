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
    pub terminal: Terminal,
    pub context: Context,
}

impl Shell {
    pub fn new() -> Result<Shell> {
        Ok(Self {
            terminal: Terminal::new(),
            context: Context::new()?,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        let mut prompt = Prompt::new();
        let mut editor = Editor::new();

        self.terminal.clear_screen()?;
        self.terminal.enter_raw_mode()?;

        loop {
            if self.context.signals.drain_child_pipe() {
                self.context.jobs.update_table(&mut self.terminal)?;
            }

            Self::update_prompt(
                &mut self.context,
                &mut self.terminal,
                &mut editor,
                &mut prompt,
            )?;

            let line = editor.read_line(&mut self.context, &mut self.terminal, &mut prompt)?;
            if line.is_empty() {
                continue;
            }

            let command = Self::parse_command(&mut self.context, &mut self.terminal, &line, true)?;
            if !Self::execute_command(&mut self.context, &mut self.terminal, command)?.0 {
                break;
            }
        }

        self.terminal.exit_raw_mode()?;

        Ok(())
    }

    fn update_prompt(
        context: &mut Context,
        terminal: &mut Terminal,
        editor: &mut Editor,
        prompt: &mut Prompt,
    ) -> Result<()> {
        prompt.update(context.update_cwd());

        if let Err(e) = editor.set_prompt(terminal) {
            terminal.println(&format!("Terminal Error: {:?}", e))?;
        }

        Ok(())
    }

    pub fn parse_command(
        context: &mut Context,
        terminal: &mut Terminal,
        line: &str,
        should_expand: bool,
    ) -> Result<Command<'static>> {
        let tokens = Tokenizer::tokenize(&line)?;

        let raw_command = Parser::parse(&tokens)?;

        if should_expand {
            let command = expander::expand(context, terminal, raw_command, &Vec::new())?;
            Ok(command)
        } else {
            if let Command::Simple {
                command,
                args,
                redirects,
                env_vars,
            } = raw_command
            {
                Ok(expander::to_owned(
                    context, terminal, command, args, redirects, env_vars,
                )?)
            } else {
                unreachable!("You can only own simple command")
            }
        }
    }

    pub fn execute_command(
        context: &mut Context,
        terminal: &mut Terminal,
        command: Command<'static>,
    ) -> Result<(bool, libc::pid_t)> {
        terminal.exit_raw_mode()?;

        let result = executor::execute(context, terminal, command, None);

        terminal.enter_raw_mode()?;

        match result {
            Ok((exit_code, pgid)) => {
                context.last_exit_code = exit_code;
                Ok((true, pgid))
            }
            Err(error) => {
                if let Some(shell_err) = error.downcast_ref::<ShellError>() {
                    if shell_err.is_exit() {
                        return Ok((false, 0));
                    }
                }
                terminal.println(&format!("{:?}", error))?;
                Ok((true, 0))
            }
        }
    }
}
