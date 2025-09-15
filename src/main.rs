use std::{
    collections::HashMap,
    env,
    io::{self, Write},
    path::PathBuf,
    process::Command,
};

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
    io::stdin()
        .read_line(&mut input)
        .expect("[ERROR] Couldn't read line");
    return input;
}

fn parse_line(line: &str) -> Vec<&str> {
    return line.trim().split_whitespace().collect();
}

fn execute(tokens: &[&str], builtins: &HashMap<&str, fn(&[&str])>) -> () {
    if tokens.is_empty() {
        return;
    }

    let program: &str = tokens[0];
    let args: &[&str] = &tokens[1..];

    if let Some(builtin) = builtins.get(program) {
        builtin(tokens);
    } else {
        let mut command: Command = Command::new(program);
        command.args(args);

        match command.status() {
            Ok(status) => {
                if !status.success() {
                    eprintln!("[SHELL ERROR] {:#?}", status.code());
                }
            }
            Err(error) => {
                eprintln!("[SHELL ERROR] {}", error);
            }
        }
    }
}

fn cd(args: &[&str]) {
    if args.len() < 2 {
        eprintln!("Error: the 'cd' command requires a path");
        return;
    }

    if let Err(error) = env::set_current_dir(args[1]) {
        eprintln!("Error: {}", error);
        return;
    }
}

fn main() {
    println!("Hello Shell!");
    let mut builtins: HashMap<&str, fn(&[&str])> = HashMap::new();
    builtins.insert("cd", cd);

    loop {
        print_prompt();
        let line: String = read_line();
        let tokens: Vec<&str> = parse_line(&line);
        execute(&tokens, &builtins);
    }
}
