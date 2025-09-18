use std::{
    collections::HashMap,
    env::{self, VarError},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    process::Command,
};

use termion::{event::Key, input::TermRead, raw::IntoRawMode, raw::RawTerminal};

fn print_prompt() -> () {
    let path: PathBuf =
        env::current_dir().expect("[SHELL ERROR] Couldn't read current working directory");

    print!("{} $> ", &path.to_string_lossy());

    io::stdout()
        .flush()
        .expect("[SHELL ERROR] Couldn't flush stdout");
}

fn read_line(config: &mut Config) -> (String, Vec<usize>) {
    let mut input: String = String::new();
    let mut stdout: RawTerminal<io::Stdout> = io::stdout()
        .into_raw_mode()
        .expect("[SHELL ERROR] Couldn't set raw mode");
    let mut substitutions_index = Vec::new();

    for character in io::stdin().keys() {
        match character.expect("[SHELL ERROR] Error while reading input") {
            Key::Char('\n') => break,

            Key::Backspace => {
                if !input.is_empty() {
                    input.pop();
                    write!(stdout, "\x1B[D \x1B[D")
                        .expect("[SHELL ERROR] Coudln't write to stdout");
                    stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
                    config.history_position = 0;
                }
            }

            Key::Char(character) => {
                input.push(character);
                write!(stdout, "{}", character).expect("[SHELL ERROR] Couldn't write to stdout");
                stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
                config.history_position = 0;

                if character == '$' {
                    substitutions_index.push(input.len() - 1);
                }
            }

            Key::Up => {
                if config.history_position > 0 {
                    config.history_position -= 1;
                }

                write!(stdout, "\r\x1B[2K").expect("[SHELL ERROR] Couldn't clear line");
                print_prompt();

                write!(stdout, "{}", config.history_vector[config.history_position])
                    .expect("[SHELL ERROR] Couldn't write to stdout");
                stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
            }

            Key::Down => {
                if config.history_position < config.history_vector.len() - 1 {
                    config.history_position += 1;
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
    write!(stdout, "\r\n").expect("[SHELL ERROR] Couldn't write to stdout");

    stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");

    return (format!("{}\n", input.trim()), substitutions_index);
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

fn exit(_args: &[&str]) -> Result<(), String> {
    println!("Goodbye!");
    std::process::exit(0);
}

//TODO: Add check to make confirm the user choice to update the variable
fn export(args: &[&str]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("Usage: export name=value".to_string());
    }

    let parts: Vec<&str> = args[1].splitn(2, '=').collect();
    if parts.len() < 2 {
        return Err("Invalid format: export name=value".to_string());
    }

    //WARN: This is not thread safe
    unsafe {
        env::set_var(parts[0], parts[1]);
    }

    Ok(())
}

fn unset(args: &[&str]) -> Result<(), String> {
    if args.len() < 2 {
        return Err("Usage: unset <name>".to_string());
    }

    unsafe {
        env::remove_var(args[1]);
    }

    Ok(())
}

struct Config {
    history_file: File,
    history_vector: Vec<String>,
    history_position: usize,
    builtins: HashMap<String, fn(&[&str]) -> Result<(), String>>,
}

fn init() -> Result<Config, io::Error> {
    const MAX_HISTORY_ENTRIES: u8 = 100;
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

    let mut history_vector: Vec<String> = Vec::new();
    if history_file.metadata()?.len() > 0 {
        let mut count: u8 = 0;
        for line in fs::read_to_string(&history_file_path)?.lines() {
            history_vector.push(line.to_string());
            count += 1;

            if count >= MAX_HISTORY_ENTRIES {
                break;
            }
        }
    }
    let history_position: usize = history_vector.len();

    let mut builtins: HashMap<String, fn(&[&str]) -> Result<(), String>> = HashMap::new();
    builtins.insert("cd".to_string(), cd);
    builtins.insert("history".to_string(), history);
    builtins.insert("exit".to_string(), exit);
    builtins.insert("export".to_string(), export);
    builtins.insert("unset".to_string(), unset);

    Ok(Config {
        history_file,
        history_vector,
        history_position,
        builtins,
    })
}

fn expand_environment_variables(
    line: &str,
    substitutions_index: Vec<usize>,
) -> Result<String, VarError> {
    let mut new_line = line.to_string();

    for &index in substitutions_index.iter().rev() {
        let end_of_substitution = new_line[index..]
            .find(|c: char| c.is_whitespace() || c == ',')
            .unwrap_or_else(|| new_line.len() - index);

        let end_index: usize = index + end_of_substitution;
        let environment_variable: &str = &new_line[index + 1..end_index];

        let substitution_value = match env::var(environment_variable) {
            Ok(value) => value,
            Err(error) => return Err(error),
        };

        new_line.replace_range(index..end_index, &substitution_value);
    }

    Ok(new_line)
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
        let (mut line, substitutions_index) = read_line(&mut config);
        if let Err(error) = config.history_file.write(&line.as_bytes()) {
            eprintln!(
                "[SHELL ERROR] Couldn't write to last command to history:\n {:#?}",
                error
            );
        }
        config.history_vector.push(line.clone());
        line = match expand_environment_variables(&line, substitutions_index) {
            Ok(new_line) => new_line,
            Err(error) => {
                println!("[SHELL ERROR] {}", error);
                continue;
            }
        };

        let tokens: Vec<&str> = parse_line(&line);
        execute(&tokens, &config.builtins);
    }
}
