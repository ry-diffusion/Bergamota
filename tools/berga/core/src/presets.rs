//! Cmdline presets: the app's `presets.json` (`[{"id": UUID, "name": .., "cmdline": ..}]`).

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub cmdline: String,
}

pub fn load(base: &Path) -> std::io::Result<Vec<Preset>> {
    let s = std::fs::read_to_string(base.join("presets.json"))?;
    serde_json::from_str(&s).map_err(std::io::Error::other)
}

/// By exact name, else by unique case-insensitive prefix.
pub fn find<'a>(presets: &'a [Preset], name: &str) -> Option<&'a Preset> {
    presets.iter().find(|p| p.name == name).or_else(|| {
        let l = name.to_lowercase();
        let mut m = presets.iter().filter(|p| p.name.to_lowercase().starts_with(&l));
        let first = m.next()?;
        m.next().is_none().then_some(first)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup() {
        let p: Vec<Preset> = serde_json::from_str(r#"[{"id":"1","name":"default @3x","cmdline":"a"},{"id":"2","name":"ios18 poc","cmdline":"b"},{"id":"3","name":"ios18 test","cmdline":"c"}]"#).unwrap();
        assert_eq!(find(&p, "default").unwrap().cmdline, "a");
        assert_eq!(find(&p, "ios18 poc").unwrap().cmdline, "b");
        assert!(find(&p, "ios18").is_none(), "ambiguous");
    }
}
