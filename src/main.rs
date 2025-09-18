use std::{
    collections::HashMap,
    env,
    fmt::write,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    process::Command,
};

use termion::{event::Key, input::TermRead, raw::IntoRawMode};

fn print_prompt() -> () {
    let path: PathBuf =
        env::current_dir().expect("[SHELL ERROR] Couldn't read current working directory");

    print!("{} $> ", &path.to_string_lossy());

    io::stdout()
        .flush()
        .expect("[SHELL ERROR] Couldn't flush stdout");
}

fn read_line(config: &mut Config) -> String {
    let mut input: String = String::new();
    let mut stdout = io::stdout()
        .into_raw_mode()
        .expect("[SHELL ERROR] Couldn't set raw mode");

    for character in io::stdin().keys() {
        match character.expect("[SHELL ERROR] Error while reading input") {
            Key::Char('\n') => break,

            Key::Backspace => {
                input.pop();
                write!(stdout, "\x1B[D \x1B[D").expect("[SHELL ERROR] Coudln't write to stdout");
                stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
                config.history_position = 0;
            }

            Key::Char(character) => {
                input.push(character);
                write!(stdout, "{}", character).expect("[SHELL ERROR] Couldn't write to stdout");
                stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
                config.history_position = 0;
            }

            //TODO: This arrow navigation is a bit weird, but it works
            Key::Up => {
                if config.history_position < config.history_vector.len() - 1
                    && config.history_position != 0
                {
                    config.history_position += 1;
                }

                write!(stdout, "\r\x1B[2K").expect("[SHELL ERROR] Couldn't clear line");
                print_prompt();

                write!(stdout, "{}", config.history_vector[config.history_position])
                    .expect("[SHELL ERROR] Couldn't write to stdout");
                stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");

                if config.history_position == 0 {
                    config.history_position += 1;
                }
            }

            Key::Down => {
                if config.history_position > 0 {
                    config.history_position -= 1;
                }

                write!(stdout, "\r\x1B[2K").expect("[SHELL ERROR] Couldn't clear line");
                print_prompt();

                write!(stdout, "{}", config.history_vector[config.history_position])
                    .expect("[SHELL ERROR] Couldn't write to stdout");
                stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
            }

            _ => {}
        }
    }
    write!(stdout, "\n").expect("[SHELL ERROR] Couldn't write to stdout");

    return format!("{}\n", input.trim());
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
    history_vector: Vec<String>,
    history_position: usize,
    builtins: HashMap<String, fn(&[&str]) -> Result<(), String>>,
}

fn init() -> Result<Config, io::Error> {
    //const MAX_HISTORY_ENTRIES: u8 = 100;
    const HISTORY_FOLDER_PATH: &str = "src/history";

    if !fs::exists(HISTORY_FOLDER_PATH)? {
        fs::create_dir(HISTORY_FOLDER_PATH)?;
    }

    let history_file_path: String = format!("{}/history.txt", HISTORY_FOLDER_PATH);
    let history_file: File = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(&history_file_path)?;

    //TODO: Improve perfromances by not loading all the file in memory
    let history_position: usize = 0;
    let mut history_vector: Vec<String>;
    if history_file.metadata()?.len() > 0 {
        history_vector = fs::read_to_string(&history_file_path)?
            .lines()
            .map(|line| line.to_string())
            .collect();
        history_vector.reverse();
    } else {
        history_vector = Vec::new();
    }

    let mut builtins: HashMap<String, fn(&[&str]) -> Result<(), String>> = HashMap::new();
    builtins.insert("cd".to_string(), cd);
    builtins.insert("history".to_string(), history);

    Ok(Config {
        history_file,
        history_vector,
        history_position,
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
        let line: String = read_line(&mut config);
        if let Err(error) = config.history_file.write(&line.as_bytes()) {
            eprintln!(
                "[SHELL ERROR] Couldn't write to last command to history:\n {:#?}",
                error
            );
        }
        config.history_vector.push(line.clone());

        let tokens: Vec<&str> = parse_line(&line);
        execute(&tokens, &config.builtins);
    }
}
