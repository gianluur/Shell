use std::collections::HashMap;

#[derive(Clone)]
pub struct Aliases {
    aliases: HashMap<String, String>,
}

impl Aliases {
    pub fn new() -> Self {
        Self {
            aliases: HashMap::new(),
        }
    }

    pub fn add(&mut self, name: String, value: String) {
        self.aliases.insert(name, value);
    }

    pub fn remove(&mut self, name: &str) {
        self.aliases.remove(name);
    }

    pub fn get(&self, name: &str) -> Option<&String> {
        self.aliases.get(name)
    }

    pub fn get_map(&self) -> &HashMap<String, String> {
        &self.aliases
    }
}
