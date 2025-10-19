use std::{
    collections::HashMap,
    env::{self, VarError},
    fs::{self, File, OpenOptions},
    io::{self, Seek, Write},
    path::PathBuf,
    process::Command,
    usize,
};

use chrono::Utc;
use termion::{
    cursor::DetectCursorPos,
    event::Key,
    input::TermRead,
    raw::{IntoRawMode, RawTerminal},
};

fn get_cwd() -> PathBuf {
    return env::current_dir().expect("[SHELL ERROR] Couldn't read current working directory");
}

fn get_prompt_length() -> usize {
    return get_cwd().to_str().unwrap().len() + 4; //<-- 4 is for the ' $> ' prompt
}

fn print_prompt(line_status: Option<&LineStatus>) -> () {
    let path: PathBuf = get_cwd();

    //TODO: I believe this statement can be simplified
    match line_status {
        Some(status) => {
            if *status == LineStatus::OK {
                print!("{} $> ", &path.to_string_lossy());
            } else {
                print!("> ");
            }
        }
        None => print!("{} $> ", &path.to_string_lossy()),
    }

    io::stdout()
        .flush()
        .expect("[SHELL ERROR] Couldn't flush stdout");
}

fn recall_command(stdout: &mut RawTerminal<io::Stdout>, config: &mut Config) -> usize {
    let commands = config.history_vector[config.history_position]
        .command
        .lines()
        .enumerate();

    write!(stdout, "\r\x1B[2K").expect("[SHELL ERROR] Couldn't clear line");
    print_prompt(None);

    let mut was_multi_line: usize = 1;
    for (i, line) in commands {
        if i == 0 {
            write!(stdout, "{}", line).expect("[SHELL ERROR] Couldn't write to stdout"); // first line after prompt
            stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
        } else {
            write!(stdout, "\r\n{}", line).expect("[SHELL ERROR] Couldn't write to stdout"); // continuation lines with a marker
            stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
            was_multi_line += 1;
        }
    }

    return was_multi_line;
}

fn clear_lines(stdout: &mut RawTerminal<io::Stdout>, lines_printed: usize) {
    for _ in 1..lines_printed {
        write!(stdout, "\x1b[2K\x1b[1A").expect("[SHELL ERROR] Couldn't move up and clear line");
        stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
    }
}

fn get_cursor_position(stdout: &mut RawTerminal<io::Stdout>) -> (u16, u16) {
    return stdout
        .cursor_pos()
        .expect("[SHELL ERROR] Couldn't get cursor position");
}

fn read_line(config: &mut Config) -> (String, Vec<usize>) {
    let mut input: String = String::new();
    let mut input_current_index: usize = input.len();

    let mut stdout: RawTerminal<io::Stdout> = io::stdout()
        .into_raw_mode()
        .expect("[SHELL ERROR] Couldn't set raw mode");
    let mut substitutions_index = Vec::new();

    let mut should_escape: bool = false;
    let mut escape_position: usize = usize::MAX;
    let mut lines_printed: usize = 1;

    for character in io::stdin().keys() {
        match character.expect("[SHELL ERROR] Error while reading input") {
            Key::Char('\n') => {
                if should_escape {
                    input.remove(escape_position);
                }
                break;
            }

            Key::Backspace => {
                if !input.is_empty() {
                    // let cursor_pos: (u16, u16) = get_cursor_position(&mut stdout);

                    input.remove(input_current_index - 1);
                    if input.len() > 0 && input_current_index > 0 {
                        input_current_index -= 1;
                    }

                    if input_current_index != input.len() {
                        write!(
                            stdout,
                            "\x1B[D \x1B[D\x1b[K{}\x1b[{}D",
                            &input[input_current_index..],
                            input.len() - input_current_index
                        )
                        .expect("[SHELL ERROR] Couldn't write to stdout");
                        stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
                    } else {
                        write!(stdout, "\x1B[D \x1B[D")
                            .expect("[SHELL ERROR] Couldn't write to stdout");
                        stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
                    }

                    let cursor_pos: (u16, u16) = get_cursor_position(&mut stdout);

                    let lines: std::str::Lines<'_> = input.lines();

                    if cursor_pos.0 == 1 && input.len() > 0 && lines.clone().count() == 1 {
                        write!(stdout, "\x1B[1A\x1B[{}C", input.len() + get_prompt_length())
                            .expect("[SHELL ERROR] Couldn't write to stdout");
                    }

                    if cursor_pos.0 == 1 && input.len() > 0 && lines.clone().count() > 1 {
                        if let Some(last_line) = lines.clone().last() {
                            let last_line_len = last_line.chars().count();
                            write!(stdout, "\x1B[1A\x1B[{}C", last_line_len + 1)
                                .expect("[SHELL ERROR] Couldn't write to stdout");
                        }
                    }
                }
            }

            Key::Char(character) => {
                input.insert(input_current_index, character);
                input_current_index += 1;

                if character == '\\' {
                    should_escape = true;
                    escape_position = input.len() - 1;
                }

                if !should_escape {
                    if character == '$' {
                        substitutions_index.push(input.len() - 1);
                    }
                }

                write!(stdout, "{}", character).expect("[SHELL ERROR] Couldn't write to stdout");

                let rest: &str = &input[input_current_index..];
                write!(stdout, "{}", rest).expect("[SHELL ERROR] Couldn't write stdout");

                if !rest.is_empty() {
                    write!(stdout, "\x1B[{}D", rest.len())
                        .expect("[SHELL ERROR] Couldn't move cursor left");
                }

                stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
            }

            Key::Up => {
                if lines_printed > 1 {
                    clear_lines(&mut stdout, lines_printed);
                }

                input.clear();
                if config.history_position > 0 {
                    config.history_position -= 1;
                }

                input.push_str(&config.history_vector[config.history_position].command);
                input_current_index = input.len();
                lines_printed = recall_command(&mut stdout, config);
            }

            Key::Down => {
                if lines_printed > 1 {
                    clear_lines(&mut stdout, lines_printed);
                }

                input.clear();
                if config.history_position < config.history_vector.len() - 1 {
                    config.history_position += 1;
                }

                input.push_str(&config.history_vector[config.history_position].command);
                input_current_index = input.len();
                lines_printed = recall_command(&mut stdout, config);
            }

            Key::Left => {
                if input.len() > 0 && input_current_index > 0 {
                    input_current_index -= 1;
                    write!(stdout, "\x1B[1D").expect("[SHELL ERROR] Couldn't write to stdout");
                    stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
                }
            }

            Key::Right => {
                if input.len() > 0 && input_current_index < input.len() {
                    input_current_index += 1;
                    write!(stdout, "\x1B[1C").expect("[SHELL ERROR] Couldn't write to stdout");
                    stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");
                }
            }

            _ => {}
        }
    }
    write!(stdout, "\r\n").expect("[SHELL ERROR] Couldn't write to stdout");
    stdout.flush().expect("[SHELL ERROR] Couldn't flush stdout");

    return (format!("{}", input.trim()), substitutions_index);
}

#[derive(PartialEq)]
enum LineStatus {
    OK,
    INCOMPLETE,
}

fn parse_line(line: &str, line_status: &mut LineStatus) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current_token: String = String::new();

    let mut inside_string: bool = match line_status {
        LineStatus::INCOMPLETE => true,
        LineStatus::OK => false,
    };

    for character in line.trim().chars() {
        if character == '\"' || character == '\'' {
            if !inside_string {
                inside_string = true
            } else {
                tokens.push(current_token.clone());
                inside_string = false;
                current_token.clear();
            }
        } else if character.is_whitespace() && !inside_string {
            if !current_token.is_empty() {
                tokens.push(current_token.clone());
                current_token.clear();
            }
        } else {
            current_token.push(character);
        }
    }

    if inside_string {
        *line_status = LineStatus::INCOMPLETE;
        current_token.push('\n');
        tokens.push(current_token);
    } else {
        if !current_token.is_empty() {
            tokens.push(current_token);
        }
        *line_status = LineStatus::OK;
    }

    tokens
}

fn execute(
    tokens: &Vec<String>,
    builtins: &HashMap<String, fn(&Vec<String>) -> Result<(), String>>,
) -> Option<i32> {
    if tokens.is_empty() {
        return None;
    }

    let program: &str = &tokens[0];
    let args: &[String] = &tokens[1..];

    if let Some(builtin) = builtins.get(program) {
        if let Err(error) = builtin(tokens) {
            eprintln!("[SHELL ERROR] {:#?}", error);
            return Some(1);
        }
        return Some(0);
    } else {
        let mut command: Command = Command::new(program);
        command.args(args);

        match command.status() {
            Ok(status) => {
                let output: Option<i32> = status.code();
                if !status.success() {
                    eprintln!("[SHELL ERROR] {:?}", output);
                }
                return output;
            }
            Err(_) => {
                eprintln!("[SHELL] Command '{}' wasn't found", &program);
                return None;
            }
        }
    }
}

fn cd(args: &Vec<String>) -> Result<(), String> {
    let directory: String = match args.len() < 2 {
        true => match env::var("HOME") {
            Ok(var) => var,
            Err(error) => return Err(format!("Error: {}", error)),
        },
        false => args[1].to_string(),
    };

    if let Err(error) = env::set_current_dir(directory) {
        return Err(format!("Error: {}", error));
    }

    Ok(())
}

fn history(args: &Vec<String>) -> Result<(), String> {
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

fn exit(_args: &Vec<String>) -> Result<(), String> {
    println!("Goodbye!");
    std::process::exit(0);
}

fn export(args: &Vec<String>) -> Result<(), String> {
    if args.len() < 2 {
        for (key, value) in env::vars() {
            println!("{}={}", key, value);
        }
        return Ok(());
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

fn unset(args: &Vec<String>) -> Result<(), String> {
    if args.len() < 2 {
        return Err("Usage: unset <name>".to_string());
    }

    unsafe {
        env::remove_var(&args[1]);
    }

    Ok(())
}

struct Config {
    history_file: File,
    history_vector: Vec<HistoryEntry>,
    history_position: usize,
    builtins: HashMap<String, fn(&Vec<String>) -> Result<(), String>>,
}

fn init() -> Result<Config, io::Error> {
    const HISTORY_FOLDER_PATH: &str = "src/history";

    if !fs::exists(HISTORY_FOLDER_PATH)? {
        fs::create_dir(HISTORY_FOLDER_PATH)?;
    }

    let history_file_path: String = format!("{}/history.json", HISTORY_FOLDER_PATH);
    let history_file: File = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(&history_file_path)?;

    let history_vector: Vec<HistoryEntry> = if history_file.metadata()?.len() > 0 {
        let contents: String = fs::read_to_string(&history_file_path)?;
        serde_json::from_str(&contents).unwrap_or_default()
    } else {
        Vec::new()
    };

    let history_position: usize = history_vector.len();

    let mut builtins: HashMap<String, fn(&Vec<String>) -> Result<(), String>> = HashMap::new();
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
            .find(|c: char| c.is_whitespace() || c == ',' || c == '\'' || c == '\"')
            .unwrap_or_else(|| new_line.len() - index);

        let end_index: usize = index + end_of_substitution;
        let environment_variable: &str = &new_line[index + 1..end_index];

        let substitution_value: String = match env::var(environment_variable) {
            Ok(value) => value,
            Err(error) => return Err(error),
        };

        new_line.replace_range(index..end_index, &substitution_value);
    }

    Ok(new_line)
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone)]
struct HistoryEntry {
    command: String,
    exit_code: Option<i32>,
    time: String,
    cwd: PathBuf,
}

fn add_history_entry(entry: HistoryEntry, config: &mut Config) {
    config.history_vector.push(entry);

    config.history_file.set_len(0).unwrap();
    config
        .history_file
        .seek(std::io::SeekFrom::Start(0))
        .unwrap();

    serde_json::to_writer_pretty(&mut config.history_file, &config.history_vector).unwrap();
    config.history_file.flush().unwrap();
}

fn main() {
    let mut config: Config = match init() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("[SHELL ERROR] Couldn't initialize shell properly {}", error);
            return;
        }
    };

    loop {
        let mut entry: HistoryEntry = HistoryEntry {
            command: String::new(),
            exit_code: None,
            time: String::new(),
            cwd: get_cwd(),
        };

        let mut tokens: Vec<String> = Vec::new();
        let mut line_status: LineStatus = LineStatus::OK;
        loop {
            print_prompt(Some(&line_status));

            let (mut line, substitutions_index): (String, Vec<usize>) = read_line(&mut config);
            if line.len() <= 1 {
                continue;
            }

            entry.command.push_str(&line);
            entry.command.push('\n');

            line = match expand_environment_variables(&line, substitutions_index) {
                Ok(new_line) => new_line,
                Err(error) => {
                    println!("[SHELL ERROR] {}", error);
                    continue;
                }
            };

            let current_tokens: Vec<String> = parse_line(&line, &mut line_status);

            if tokens.is_empty() {
                tokens.extend_from_slice(&current_tokens);
            } else {
                if !current_tokens.is_empty() {
                    if let Some(last) = tokens.last_mut() {
                        last.push_str(&current_tokens[0]);
                    }
                    if current_tokens.len() > 1 {
                        tokens.extend_from_slice(&current_tokens[1..]);
                    }
                }
            }

            if line_status == LineStatus::OK {
                entry.command = entry.command.trim_end().to_owned();
                break;
            }
        }

        entry.exit_code = execute(&tokens, &config.builtins);
        entry.time = Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        add_history_entry(entry, &mut config);
    }
}
