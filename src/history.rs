use anyhow::{Context, Ok, Result};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

pub struct History {
    file: File,
    pub row: usize,
    pub current: Vec<String>,
}

impl History {
    pub fn new() -> Result<Self> {
        let home_dir = env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let path = PathBuf::from(home_dir).join(".rshell_history");

        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)?;

        let mut current = Vec::new();
        let content = fs::read_to_string(&path).context("Failed to open history file")?;
        content.lines().for_each(|l| current.push(l.to_string()));

        Ok(Self {
            file,
            row: current.len(),
            current,
        })
    }

    pub fn push(&mut self, command: String) -> Result<()> {
        writeln!(self.file, "{}", command)?;
        self.file.flush()?;

        self.current.push(command);

        Ok(())
    }
}
