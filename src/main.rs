//main.rs

use rshell::shell::Shell;

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
