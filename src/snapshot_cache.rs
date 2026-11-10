use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::model::{Meter, ProviderSnapshot, ProviderStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Cursor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedSnapshot {
    plan: Option<String>,
    meters: Vec<Meter>,
    fetched_at: DateTime<Utc>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    claude: Option<CachedSnapshot>,
    cursor: Option<CachedSnapshot>,
}

impl CacheFile {
    fn slot_mut(&mut self, provider: Provider) -> &mut Option<CachedSnapshot> {
        match provider {
            Provider::Claude => &mut self.claude,
            Provider::Cursor => &mut self.cursor,
        }
    }

    fn slot(&self, provider: Provider) -> &Option<CachedSnapshot> {
        match provider {
            Provider::Claude => &self.claude,
            Provider::Cursor => &self.cursor,
        }
    }
}

pub struct LastGood {
    file: CacheFile,
    path: Option<PathBuf>,
}

impl LastGood {
    /// Loads `<config_dir>/ai-usage-widget/last_good.json`; missing or corrupt file → empty.
    pub fn load() -> LastGood {
        let path = crate::config::config_path().map(|p| p.with_file_name("last_good.json"));
        Self::load_from(path)
    }

    pub(crate) fn load_from(path: Option<PathBuf>) -> LastGood {
        let file = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default();
        LastGood { file, path }
    }

    pub fn apply(
        &mut self,
        provider: Provider,
        snap: ProviderSnapshot,
        now: DateTime<Utc>,
    ) -> ProviderSnapshot {
        match &snap.status {
            ProviderStatus::Ok if !snap.meters.is_empty() => {
                self.file.slot_mut(provider).replace(CachedSnapshot {
                    plan: snap.plan.clone(),
                    meters: snap.meters.clone(),
                    fetched_at: snap.fetched_at,
                });
                self.write_file();
                snap
            }
            ProviderStatus::Ok | ProviderStatus::NotSignedIn(_) => snap,
            ProviderStatus::Error(msg) | ProviderStatus::Stale(msg) => {
                let Some(stored) = self.file.slot(provider).clone() else {
                    return snap;
                };
                let when = format_when(stored.fetched_at, now);
                let status_msg = stale_message(msg, &when);
                let meters = adjust_meters(stored.meters, now);
                ProviderSnapshot {
                    name: snap.name,
                    plan: snap.plan.or(stored.plan),
                    meters,
                    status: ProviderStatus::Stale(status_msg),
                    fetched_at: stored.fetched_at,
                }
            }
        }
    }

    fn write_file(&self) {
        let Some(ref path) = self.path else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.file) {
            let _ = std::fs::write(path, json);
        }
    }
}

fn format_when(stored: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let stored_local = stored.with_timezone(&Local);
    let now_local = now.with_timezone(&Local);
    if stored_local.date_naive() == now_local.date_naive() {
        stored_local.format("%H:%M").to_string()
    } else {
        stored_local.format("%a %H:%M").to_string()
    }
}

fn stale_message(msg: &str, when: &str) -> String {
    const MARKER: &str = " · showing data from ";
    if msg.contains(MARKER) {
        msg.to_string()
    } else {
        format!("{msg} · showing data from {when}")
    }
}

fn adjust_meters(meters: Vec<Meter>, now: DateTime<Utc>) -> Vec<Meter> {
    meters
        .into_iter()
        .map(|mut m| {
            if m.resets_at.is_some_and(|t| t <= now) {
                m.percent = 0.0;
                m.resets_at = None;
                m.detail = Some("Window has reset since this reading".to_string());
            }
            m
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_path(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ai-usage-widget-last-good-{}-{}.json",
            std::process::id(),
            test_name
        ))
    }

    fn remove_path(path: &PathBuf) {
        let _ = std::fs::remove_file(path);
    }

    fn ok_snap(meters: Vec<Meter>, fetched_at: DateTime<Utc>) -> ProviderSnapshot {
        ProviderSnapshot {
            name: "Claude",
            plan: Some("Pro".to_string()),
            meters,
            status: ProviderStatus::Ok,
            fetched_at,
        }
    }

    #[test]
    fn ok_is_stored_and_returned_unchanged() {
        let path = temp_path("ok_is_stored_and_returned_unchanged");
        remove_path(&path);
        let now = Utc::now();
        let meter = Meter {
            label: "weekly".to_string(),
            percent: 42.0,
            detail: None,
            resets_at: None,
        };
        let snap = ok_snap(vec![meter.clone()], now);

        let mut lg = LastGood::load_from(Some(path.clone()));
        let out = lg.apply(Provider::Claude, snap.clone(), now);

        assert_eq!(out.status, ProviderStatus::Ok);
        assert_eq!(out.meters.len(), 1);
        assert_eq!(out.meters[0].percent, 42.0);
        assert!(lg.file.claude.is_some());
        remove_path(&path);
    }

    #[test]
    fn error_after_ok_returns_stale_old_meters() {
        let path = temp_path("error_after_ok_returns_stale_old_meters");
        remove_path(&path);
        let stored_at = Utc::now() - chrono::Duration::minutes(5);
        let now = Utc::now();
        let meter = Meter {
            label: "weekly".to_string(),
            percent: 55.0,
            detail: None,
            resets_at: None,
        };
        let mut lg = LastGood::load_from(Some(path.clone()));
        lg.apply(
            Provider::Claude,
            ok_snap(vec![meter.clone()], stored_at),
            stored_at,
        );

        let err = ProviderSnapshot {
            name: "Claude",
            plan: None,
            meters: vec![],
            status: ProviderStatus::Error("HTTP 429".to_string()),
            fetched_at: now,
        };
        let out = lg.apply(Provider::Claude, err, now);

        assert!(matches!(out.status, ProviderStatus::Stale(ref m) if m.contains("HTTP 429") && m.contains("showing data from")));
        assert_eq!(out.meters.len(), 1);
        assert_eq!(out.meters[0].percent, 55.0);
        assert_eq!(out.fetched_at, stored_at);
        remove_path(&path);
    }

    #[test]
    fn expired_window_shows_zero() {
        let path = temp_path("expired_window_shows_zero");
        remove_path(&path);
        let stored_at = Utc::now() - chrono::Duration::hours(2);
        let now = Utc::now();
        let reset_at = now - chrono::Duration::minutes(1);
        let meter = Meter {
            label: "session".to_string(),
            percent: 80.0,
            detail: Some("busy".to_string()),
            resets_at: Some(reset_at),
        };
        let mut lg = LastGood::load_from(Some(path.clone()));
        lg.apply(
            Provider::Cursor,
            ProviderSnapshot {
                name: "Cursor",
                plan: None,
                meters: vec![meter],
                status: ProviderStatus::Ok,
                fetched_at: stored_at,
            },
            stored_at,
        );

        let err = ProviderSnapshot {
            name: "Cursor",
            plan: None,
            meters: vec![],
            status: ProviderStatus::Error("network".to_string()),
            fetched_at: now,
        };
        let out = lg.apply(Provider::Cursor, err, now);

        assert_eq!(out.meters.len(), 1);
        assert_eq!(out.meters[0].percent, 0.0);
        assert!(out.meters[0].resets_at.is_none());
        assert_eq!(
            out.meters[0].detail.as_deref(),
            Some("Window has reset since this reading")
        );
        remove_path(&path);
    }

    #[test]
    fn not_signed_in_passes_through() {
        let path = temp_path("not_signed_in_passes_through");
        remove_path(&path);
        let now = Utc::now();
        let mut lg = LastGood::load_from(Some(path.clone()));
        lg.apply(
            Provider::Claude,
            ok_snap(
                vec![Meter {
                    label: "x".to_string(),
                    percent: 1.0,
                    detail: None,
                    resets_at: None,
                }],
                now,
            ),
            now,
        );

        let snap = ProviderSnapshot {
            name: "Claude",
            plan: None,
            meters: vec![],
            status: ProviderStatus::NotSignedIn("no cookie".to_string()),
            fetched_at: now,
        };
        let out = lg.apply(Provider::Claude, snap.clone(), now);
        assert_eq!(out.status, snap.status);
        assert!(out.meters.is_empty());
        remove_path(&path);
    }

    #[test]
    fn error_without_cache_passes_through() {
        let path = temp_path("error_without_cache_passes_through");
        remove_path(&path);
        let now = Utc::now();
        let mut lg = LastGood::load_from(Some(path.clone()));
        let snap = ProviderSnapshot {
            name: "Claude",
            plan: None,
            meters: vec![],
            status: ProviderStatus::Error("fail".to_string()),
            fetched_at: now,
        };
        let out = lg.apply(Provider::Claude, snap.clone(), now);
        assert_eq!(out.status, snap.status);
        remove_path(&path);
    }

    #[test]
    fn persists_across_load() {
        let path = temp_path("persists_across_load");
        remove_path(&path);
        let stored_at = Utc::now() - chrono::Duration::minutes(10);
        let now = Utc::now();
        let meter = Meter {
            label: "weekly".to_string(),
            percent: 33.0,
            detail: None,
            resets_at: None,
        };

        {
            let mut lg = LastGood::load_from(Some(path.clone()));
            lg.apply(
                Provider::Claude,
                ok_snap(vec![meter.clone()], stored_at),
                stored_at,
            );
        }

        let mut lg = LastGood::load_from(Some(path.clone()));
        let err = ProviderSnapshot {
            name: "Claude",
            plan: None,
            meters: vec![],
            status: ProviderStatus::Error("offline".to_string()),
            fetched_at: now,
        };
        let out = lg.apply(Provider::Claude, err, now);
        assert!(matches!(out.status, ProviderStatus::Stale(_)));
        assert_eq!(out.meters[0].percent, 33.0);
        remove_path(&path);
    }
}
