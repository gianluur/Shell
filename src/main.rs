//main.rs

mod builtins;
mod context;
mod editor;
mod error;
mod executor;
mod expander;
mod history;
mod jobs;
mod parser;
mod prompt;
mod shell;
mod signals;
mod terminal;
mod tokenizer;

use shell::Shell;

fn main() {
    let mut shell = match Shell::new() {
        Ok(shell) => shell,
        Err(error) => {
            eprintln!("Failed to start up RShell: {:?}", error);
            std::process::exit(1)
        }
    };

    if let Err(e) = shell.run() {
        eprintln!("Critical Shell Error: {:?}", e);
        std::process::exit(1);
    }
}
