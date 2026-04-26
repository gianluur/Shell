//prompt.rs

use std::path::PathBuf;

pub struct Prompt {
    pub message: String,
}

//TODO: For now we do a static prompt, after i finished everything i will add the ability to customize it
impl Prompt {
    pub fn new() -> Self {
        Self {
            message: String::new(),
        }
    }

    pub fn update(&mut self, directory: &PathBuf) {
        self.message = format!("{} >> ", directory.display());
    }

    pub fn len(&self) -> usize {
        self.message.len()
    }
}
