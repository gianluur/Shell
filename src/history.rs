// history.rs

use anyhow::{Context, Result};
use std::{
    env,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

pub struct History {
    file: Option<File>,
    pub row: usize,
    pub current: Vec<String>,
}

impl Clone for History {
    fn clone(&self) -> Self {
        Self {
            file: None,
            row: self.row,
            current: self.current.clone(),
        }
    }
}

impl History {
    pub fn new() -> Result<Self> {
        let home_dir = env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let path = PathBuf::from(home_dir).join(".rshell_history");

        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)
            .context("Failed to read history file")?;

        let mut current = Vec::new();
        let mut content = String::new();
        file.read_to_string(&mut content)
            .context("Failed to read history file")?;

        content.lines().for_each(|l| current.push(l.to_string()));

        Ok(Self {
            file: Some(file),
            row: current.len(),
            current,
        })
    }

    pub fn dummy() -> Self {
        Self {
            file: None,
            current: Vec::new(),
            row: 0,
        }
    }

    pub fn push(&mut self, command: String) -> Result<()> {
        if let Some(ref mut file) = self.file {
            writeln!(file, "{}", command)?;
            file.flush()?;
        }

        self.current.push(command);

        Ok(())
    }
}
