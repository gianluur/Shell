use std::{
    collections::HashMap,
    env,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    process::Command,
};

use termion::{event::Key, input::TermRead};

fn print_prompt() -> () {
    let path: PathBuf =
        env::current_dir().expect("[SHELL ERROR] Couldn't read current working directory");

    print!("{} $> ", &path.to_string_lossy());

    io::stdout()
        .flush()
        .expect("[SHELL ERROR] Couldn't flush stdout");
}

fn read_line() -> String {
    let mut input: String = String::new();
    let mut stdout = io::stdout().lock();

    for character in io::stdin().keys() {
        match character.expect("[SHELL ERROR] Error while reading input") {
            Key::Char('\n') => break,

            Key::Backspace => {
                input.pop();
                write!(stdout, "\x1B[D \x1B[D").expect("[SHELL ERROR] Coudln't write to stdout");
                stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
            }

            Key::Char(character) => {
                input.push(character);
            }

            _ => {}
        }
    }

    return input.trim().to_string();
}

fn parse_line(line: &str) -> Vec<&str> {
    return line.trim().split_whitespace().collect();
}

fn execute(tokens: &[&str], builtins: &HashMap<String, fn(&[&str]) -> Result<(), String>>) -> () {
    if tokens.is_empty() {
        return;
    }

    let program: &str = tokens[0];
    let args: &[&str] = &tokens[1..];

    if let Some(builtin) = builtins.get(program) {
        if let Err(error) = builtin(tokens) {
            eprintln!("[SHELL ERROR] {:#?}", error);
        }
    } else {
        let mut command: Command = Command::new(program);
        command.args(args);

        match command.status() {
            Ok(status) => {
                if !status.success() {
                    eprintln!("[SHELL ERROR] {:#?}", status.code());
                }
            }
            Err(_) => {
                eprintln!("[SHELL] Command '{}' wasn't found", &program);
            }
        }
    }
}

fn cd(args: &[&str]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("Error: the 'cd' command requires a path".to_string());
    }

    if let Err(error) = env::set_current_dir(args[1]) {
        return Err(format!("Error: {}", error));
    }

    Ok(())
}

fn history(args: &[&str]) -> Result<(), String> {
    println!("History: ");
    let contents = match fs::read_to_string(&args[1]) {
        Ok(contents) => contents,
        Err(error) => {
            return Err(format!("Error: {}", error));
        }
    };

    println!("{}", contents);
    Ok(())
}

struct Config {
    history_file: File,
    builtins: HashMap<String, fn(&[&str]) -> Result<(), String>>,
}

fn init() -> Result<Config, io::Error> {
    if !fs::exists("src/history")? {
        fs::create_dir("src/history")?;
    }

    let history_file: File = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open("src/history/history.txt")?;

    let mut builtins: HashMap<String, fn(&[&str]) -> Result<(), String>> = HashMap::new();
    builtins.insert("cd".to_string(), cd);
    builtins.insert("history".to_string(), history);

    Ok(Config {
        history_file,
        builtins,
    })
}

fn main() {
    println!("Hello Shell!");

    let mut config: Config = match init() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("[SHELL ERROR] Couldn't initialize shell properly {}", error);
            return;
        }
    };

    loop {
        print_prompt();
        let line: String = read_line();
        if let Err(error) = config.history_file.write(line.as_bytes()) {
            eprintln!(
                "[SHELL ERROR] Couldn't write to last command to history:\n {:#?}",
                error
            );
        }

        let tokens: Vec<&str> = parse_line(&line);
        execute(&tokens, &config.builtins);
    }
}
