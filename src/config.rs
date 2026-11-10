use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    #[default]
    Vertical,
    Horizontal,
}

impl Serialize for Layout {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Layout::Vertical => serializer.serialize_str("vertical"),
            Layout::Horizontal => serializer.serialize_str("horizontal"),
        }
    }
}

impl<'de> Deserialize<'de> for Layout {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(match value {
            serde_json::Value::String(s) if s.trim().eq_ignore_ascii_case("horizontal") => {
                Layout::Horizontal
            }
            _ => Layout::Vertical,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub cursor_session_token: Option<String>,
    pub window_x: Option<f32>,
    pub window_y: Option<f32>,
    pub always_on_top: bool,
    pub poll_seconds: u64,
    /// "vertical" (default) or "horizontal" compact strip.
    pub layout: Layout,
    /// Dock a horizontal strip over the Windows taskbar (implies horizontal layout).
    pub taskbar_mode: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cursor_session_token: None,
            window_x: None,
            window_y: None,
            always_on_top: true,
            poll_seconds: 120,
            layout: Layout::Vertical,
            taskbar_mode: false,
        }
    }
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ai-usage-widget").join("config.json"))
}

pub fn load() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return Config::default(),
    };
    serde_json::from_str(&data).unwrap_or_default()
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    let path = config_path().ok_or(std::io::ErrorKind::NotFound)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(cfg)?;
    std::fs::write(path, json)
}

pub fn ensure_exists() -> std::io::Result<()> {
    let Some(path) = config_path() else {
        return Err(std::io::ErrorKind::NotFound.into());
    };
    if path.exists() {
        return Ok(());
    }
    save(&Config::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let cfg = Config::default();
        assert!(cfg.always_on_top);
        assert_eq!(cfg.poll_seconds, 120);
        assert_eq!(cfg.layout, Layout::Vertical);
        assert!(!cfg.taskbar_mode);
    }

    #[test]
    fn deserializes_partial() {
        let cfg: Config = serde_json::from_str(r#"{"poll_seconds":60}"#).unwrap();
        assert_eq!(cfg.poll_seconds, 60);
        assert!(cfg.always_on_top);
        assert!(cfg.cursor_session_token.is_none());
    }

    #[test]
    fn deserializes_layout_and_taskbar() {
        let cfg: Config =
            serde_json::from_str(r#"{"layout":"horizontal","taskbar_mode":true}"#).unwrap();
        assert_eq!(cfg.layout, Layout::Horizontal);
        assert!(cfg.taskbar_mode);
    }

    #[test]
    fn layout_is_case_insensitive() {
        let cfg: Config = serde_json::from_str(r#"{"layout":"  Horizontal "}"#).unwrap();
        assert_eq!(cfg.layout, Layout::Horizontal);
    }

    #[test]
    fn invalid_layout_keeps_other_fields() {
        let cfg: Config = serde_json::from_str(
            r#"{"layout":"diagonal","poll_seconds":60,"cursor_session_token":"abc"}"#,
        )
        .unwrap();
        assert_eq!(cfg.layout, Layout::Vertical);
        assert_eq!(cfg.poll_seconds, 60);
        assert_eq!(cfg.cursor_session_token.as_deref(), Some("abc"));
    }

    #[test]
    fn non_string_layout_is_vertical() {
        let cfg: Config = serde_json::from_str(r#"{"layout":5}"#).unwrap();
        assert_eq!(cfg.layout, Layout::Vertical);
    }

    #[test]
    fn serializes_lowercase() {
        let value = serde_json::to_value(Config {
            layout: Layout::Horizontal,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(value["layout"], "horizontal");
        assert_eq!(value["taskbar_mode"], false);
    }
}
