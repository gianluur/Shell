# RShell

RShell is an interactive command-line shell for Unix-like operating systems, built from scratch in Rust. It supports job control, pipelines, redirections, expansions, built-in commands, and a raw‑mode line editor with history. The project demonstrates how low‑level system calls (fork, execvp, pipe, sigaction, tcsetpgrp, etc.) can be combined with safe Rust abstractions to create a fully functional shell.

## Features

- **Command Parsing & Expansion**  
  Tokenizer, parser, and expander handle quoting (`'`, `"`), environment variables (`$VAR`, `${VAR}`, `$?`, `$$`, `$!`), tilde (`~`), and escape sequences.

- **Pipelines & Redirections**  
  `|`, `>`, `>>`, `<`, `2>`, `2>&1`. Both foreground and background pipelines are supported.

- **Job Control**  
  Background jobs (`&`), `jobs`, `fg`, `bg`. The shell tracks process groups, handles `SIGCHLD`, and notifies about job state changes (stopped, continued, completed).

- **Built‑in Commands**  
  `cd`, `exit`, `jobs`, `fg`, `bg`, `history`.

- **Line Editor with Raw Mode**  
  - Left/right arrow, home/end, backspace.  
  - Up/down arrows for command history.  
  - Alt + left/right for word jumping.  
  - Ctrl+C clears the current line, Ctrl+L clears the screen.  
  - History stored in `~/.rshell_history`.

- **Signal Handling**  
  The shell ignores `SIGINT`, `SIGTSTP`, `SIGTTOU`, `SIGTTIN` while it is the foreground process, but resets them to defaults for child processes. The self‑pipe trick is used to safely handle `SIGCHLD`.

- **Dynamic Prompt**  
  Shows the current working directory, e.g., `/home/user >> `.

- **Error Reporting**  
  Phase‑specific errors (tokenizer, parser, expander, executor) with user‑friendly messages.

## Building

### Prerequisites

- Rust (latest stable) and Cargo
- A Unix‑like operating system (Linux, macOS, etc.)

### Compile and Run

```bash
git clone https://github.com/gianluur/Shell.git
cd rshell
cargo build --release
./target/release/rshell
```

If you want to run it without installing:

```bash
cargo run
```

## Usage

Start RShell and you will see a prompt:

```
/home/user >>
```

Type commands as you would in bash or zsh:

```bash
echo "Hello, world!"
ls -la | grep ".rs"
cat file.txt > output.txt
sleep 10 &
jobs
fg %1
cd /tmp
cd -
```

### Built‑in Commands

| Command        | Description                                          |
|----------------|------------------------------------------------------|
| `cd [dir]`     | Change directory. `cd` alone goes to `$HOME`. `cd -` goes to `$OLDPWD`. |
| `exit`         | Exit the shell.                                      |
| `jobs`         | List background and stopped jobs.                    |
| `fg [%job]`    | Bring a background or stopped job to the foreground. |
| `bg [%job]`    | Resume a stopped job in the background.              |
| `history`      | Show command history.                                |

### Keyboard Shortcuts

| Key                     | Action                         |
|-------------------------|--------------------------------|
| Left / Right            | Move cursor within line        |
| Alt + Left / Right      | Jump to previous / next word   |
| Home / End              | Move to start / end of line    |
| Up / Down               | Navigate command history       |
| Backspace               | Delete character before cursor |
| Ctrl + C                | Clear current line             |
| Ctrl + L                | Clear screen and redraw prompt |
| Enter                   | Execute command                |

## Project Structure

| Module          | Responsibility                                             |
|-----------------|------------------------------------------------------------|
| `tokenizer`     | Splits input into tokens (words, operators, quotes).      |
| `parser`        | Builds an AST: pipelines, `&&`, `\|\|`, `;`, `&`, redirects. |
| `expander`      | Expands variables, tilde, and quotes in the AST.          |
| `executor`      | Forks processes, sets up pipes/redirections, execs commands. |
| `jobs`          | Tracks process groups, job states, and handles `waitpid`. |
| `builtins`      | Implements `cd`, `exit`, `jobs`, `fg`, `bg`, `history`.   |
| `editor`        | Raw‑mode line editor with history navigation.             |
| `terminal`      | Wraps crossterm and raw mode management.                  |
| `signals`       | Self‑pipe trick for `SIGCHLD`, ignores/restores signals.  |
| `context`       | Global shell state (directory, PGID, history, job table). |
| `history`       | Loads/saves command history to `~/.rshell_history`.       |

## Dependencies

- `anyhow` – flexible error handling
- `crossterm` – terminal manipulation and raw mode
- `libc` – raw system calls (fork, execvp, pipe, signal, waitpid, etc.)

All dependencies are listed in `Cargo.toml`.

## Limitations & Future Work

- No support for `$(command)` substitution or arithmetic expansion.
- No alias or completion system.
- The expander does not handle escaping of `$` or `\` inside double quotes fully.
- History does not support reverse search (`Ctrl+R`).

These features will be added in future versions.

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.

---