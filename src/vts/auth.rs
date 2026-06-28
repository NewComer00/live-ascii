use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use uuid::Uuid;

pub fn plugin_key(plugin_name: &str, plugin_developer: &str) -> String {
    format!("{plugin_name}|{plugin_developer}")
}

pub fn validate_plugin_identity(name: &str, developer: &str) -> Result<(), &'static str> {
    if name.len() < 3 || name.len() > 32 {
        return Err("pluginName must be 3-32 characters");
    }
    if developer.len() < 3 || developer.len() > 32 {
        return Err("pluginDeveloper must be 3-32 characters");
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct TokenStore {
    path: PathBuf,
    tokens: HashMap<String, String>,
}

impl TokenStore {
    pub fn load_default() -> Self {
        let path = default_token_path();
        let tokens = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, tokens }
    }

    pub fn get_token(&self, plugin_name: &str, plugin_developer: &str) -> Option<&str> {
        self.tokens
            .get(&plugin_key(plugin_name, plugin_developer))
            .map(|s| s.as_str())
    }

    pub fn issue_token(&mut self, plugin_name: &str, plugin_developer: &str) -> String {
        let key = plugin_key(plugin_name, plugin_developer);
        if let Some(existing) = self.tokens.get(&key) {
            return existing.clone();
        }
        let token = Uuid::new_v4().as_simple().to_string();
        self.tokens.insert(key, token.clone());
        let _ = self.save();
        token
    }

    pub fn validate_token(
        &self,
        plugin_name: &str,
        plugin_developer: &str,
        token: &str,
    ) -> bool {
        self.get_token(plugin_name, plugin_developer)
            .is_some_and(|stored| stored == token)
    }

    fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.tokens)?;
        fs::write(&self.path, json)
    }

    #[cfg(test)]
    fn load_from_map(tokens: HashMap<String, String>) -> Self {
        Self {
            path: PathBuf::from("/tmp/unused"),
            tokens,
        }
    }
}

fn default_token_path() -> PathBuf {
    dirs_home().join(".live-ascii").join("vts_tokens.json")
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_key_format() {
        assert_eq!(plugin_key("My Plugin", "Dev"), "My Plugin|Dev");
    }

    #[test]
    fn validate_identity_lengths() {
        assert!(validate_plugin_identity("abc", "def").is_ok());
        assert!(validate_plugin_identity("ab", "def").is_err());
        assert!(validate_plugin_identity("abc", "de").is_err());
    }

    #[test]
    fn token_issue_and_validate() {
        let mut store = TokenStore::load_from_map(HashMap::new());
        let token = store.issue_token("My Plugin", "Dev Name");
        assert!(store.validate_token("My Plugin", "Dev Name", &token));
        assert!(!store.validate_token("My Plugin", "Dev Name", "wrong"));
        assert_eq!(store.issue_token("My Plugin", "Dev Name"), token);
    }
}
