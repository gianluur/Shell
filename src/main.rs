//main.rs

mod builtins;
mod context;
mod editor;
mod error;
mod executor;
mod expander;
mod jobs;
mod parser;
mod prompt;
mod shell;
mod signals;
mod terminal;
mod tokenizer;

use shell::Shell;

fn main() {
    if let Err(e) = Shell::new().run() {
        eprintln!("Critical Shell Error: {:?}", e);
        std::process::exit(1);
    }
}
