use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::model::{Meter, ProviderSnapshot, ProviderStatus};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

pub fn credentials_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join(".credentials.json"));
        }
    }
    dirs::home_dir().map(|h| h.join(".claude").join(".credentials.json"))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialsFile {
    claude_ai_oauth: Option<ClaudeAiOauth>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct ClaudeAiOauth {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<i64>,
    scopes: Option<Vec<String>>,
    subscription_type: Option<String>,
    rate_limit_tier: Option<String>,
}

fn clamp_percent(p: f32) -> f32 {
    p.clamp(0.0, 100.0)
}

fn parse_resets_at(value: &serde_json::Value) -> Option<DateTime<Utc>> {
    value
        .get("resets_at")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

fn meter_from_window(label: &str, obj: &serde_json::Value) -> Option<Meter> {
    if obj.is_null() {
        return None;
    }
    let utilization = obj.get("utilization")?.as_f64()? as f32;
    Some(Meter {
        label: label.to_string(),
        percent: clamp_percent(utilization),
        detail: None,
        resets_at: parse_resets_at(obj),
    })
}

fn meter_from_optional_window(label: &str, value: &serde_json::Value) -> Option<Meter> {
    let key = match label {
        "7d Opus" => "seven_day_opus",
        "7d Sonnet" => "seven_day_sonnet",
        _ => return None,
    };
    let obj = value.get(key)?;
    if obj.is_null() {
        return None;
    }
    let utilization = obj.get("utilization")?;
    if !utilization.is_number() {
        return None;
    }
    let utilization = utilization.as_f64()? as f32;
    Some(Meter {
        label: label.to_string(),
        percent: clamp_percent(utilization),
        detail: None,
        resets_at: parse_resets_at(obj),
    })
}

pub fn parse_usage(json: &str) -> Result<Vec<Meter>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "invalid usage JSON".to_string())?;

    let mut meters = Vec::new();

    if let Some(obj) = v.get("five_hour") {
        if let Some(m) = meter_from_window("5h", obj) {
            meters.push(m);
        }
    }

    if let Some(obj) = v.get("seven_day") {
        if let Some(m) = meter_from_window("7d", obj) {
            meters.push(m);
        }
    }

    if let Some(m) = meter_from_optional_window("7d Opus", &v) {
        meters.push(m);
    }

    if let Some(m) = meter_from_optional_window("7d Sonnet", &v) {
        meters.push(m);
    }

    if let Some(extra) = v.get("extra_usage") {
        if !extra.is_null() {
            let enabled = extra
                .get("is_enabled")
                .and_then(|e| e.as_bool())
                .unwrap_or(false);
            if enabled {
                if let Some(util) = extra.get("utilization") {
                    if util.is_number() {
                        if let Some(p) = util.as_f64() {
                            meters.push(Meter {
                                label: "Extra".to_string(),
                                percent: clamp_percent(p as f32),
                                detail: None,
                                resets_at: None,
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(meters)
}

pub(crate) fn plan_label(subscription_type: &str) -> String {
    let s = subscription_type.trim();
    if s.is_empty() {
        return String::new();
    }
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            upper + chars.as_str()
        }
    }
}

fn read_credentials() -> Result<ClaudeAiOauth, ProviderStatus> {
    let path = match credentials_path() {
        Some(p) => p,
        None => {
            return Err(ProviderStatus::NotSignedIn(
                "Sign in to Claude Code".to_string(),
            ));
        }
    };

    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => {
            return Err(ProviderStatus::NotSignedIn(
                "Sign in to Claude Code".to_string(),
            ));
        }
    };

    let file: CredentialsFile = match serde_json::from_str(&data) {
        Ok(f) => f,
        Err(_) => {
            return Err(ProviderStatus::Error(
                "Unreadable Claude credentials".to_string(),
            ));
        }
    };

    let oauth = match file.claude_ai_oauth {
        Some(o) => o,
        None => {
            return Err(ProviderStatus::Error(
                "Unreadable Claude credentials".to_string(),
            ));
        }
    };

    Ok(oauth)
}

/// Shortest gap between two calls to the usage endpoint, even on manual refresh.
const MIN_GAP: Duration = Duration::from_secs(60);
/// First backoff after a 429 when the server sends no Retry-After.
const BACKOFF_START: Duration = Duration::from_secs(120);
const BACKOFF_MAX: Duration = Duration::from_secs(15 * 60);

enum FetchOutcome {
    Snapshot(ProviderSnapshot),
    RateLimited {
        plan: Option<String>,
        retry_after: Option<Duration>,
    },
}

fn format_wait(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 60 {
        format!("{}m", secs.div_ceil(60))
    } else {
        format!("{secs}s")
    }
}

/// One-shot fetch (used by `--once`). A 429 is reported as an error.
pub fn fetch() -> ProviderSnapshot {
    match fetch_once() {
        FetchOutcome::Snapshot(s) => s,
        FetchOutcome::RateLimited { plan, retry_after } => ProviderSnapshot {
            name: "Claude",
            plan,
            meters: vec![],
            status: ProviderStatus::Error(match retry_after {
                Some(d) => format!("Rate limited - retry in {}", format_wait(d)),
                None => "Rate limited (HTTP 429)".to_string(),
            }),
            fetched_at: Utc::now(),
        },
    }
}

/// Stateful poller for the widget: enforces a minimum gap between calls,
/// backs off on HTTP 429 (honouring Retry-After), and keeps showing the
/// last good meters while rate limited.
pub struct Poller {
    last: Option<ProviderSnapshot>,
    last_good: Option<ProviderSnapshot>,
    last_call: Option<Instant>,
    blocked_until: Option<Instant>,
    backoff: Duration,
}

impl Poller {
    pub fn new() -> Self {
        Self {
            last: None,
            last_good: None,
            last_call: None,
            blocked_until: None,
            backoff: BACKOFF_START,
        }
    }

    pub fn poll(&mut self) -> ProviderSnapshot {
        let now = Instant::now();
        let too_soon = self.last_call.is_some_and(|t| now.duration_since(t) < MIN_GAP);
        let blocked = self.blocked_until.is_some_and(|t| now < t);
        if let (true, Some(last)) = (too_soon || blocked, &self.last) {
            return last.clone();
        }

        self.last_call = Some(now);
        let snap = match fetch_once() {
            FetchOutcome::Snapshot(s) => {
                self.blocked_until = None;
                self.backoff = BACKOFF_START;
                if s.status == ProviderStatus::Ok {
                    self.last_good = Some(s.clone());
                }
                s
            }
            FetchOutcome::RateLimited { plan, retry_after } => {
                let wait = retry_after
                    .map(|d| d.clamp(MIN_GAP, BACKOFF_MAX))
                    .unwrap_or(self.backoff);
                self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
                self.blocked_until = Some(now + wait);
                let msg = format!("Rate limited - retry in {}", format_wait(wait));
                match &self.last_good {
                    Some(good) => ProviderSnapshot {
                        status: ProviderStatus::Stale(msg),
                        ..good.clone()
                    },
                    None => ProviderSnapshot {
                        name: "Claude",
                        plan,
                        meters: vec![],
                        status: ProviderStatus::Error(msg),
                        fetched_at: Utc::now(),
                    },
                }
            }
        };
        self.last = Some(snap.clone());
        snap
    }
}

fn fetch_once() -> FetchOutcome {
    let fetched_at = Utc::now();
    let snap = |plan: Option<String>, meters: Vec<Meter>, status: ProviderStatus| {
        FetchOutcome::Snapshot(ProviderSnapshot {
            name: "Claude",
            plan,
            meters,
            status,
            fetched_at,
        })
    };

    let oauth = match read_credentials() {
        Ok(o) => o,
        Err(status) => return snap(None, vec![], status),
    };

    let plan = oauth
        .subscription_type
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(plan_label);

    if let Some(expires_at) = oauth.expires_at {
        if expires_at <= Utc::now().timestamp_millis() {
            return snap(
                plan,
                vec![],
                ProviderStatus::Error("Token expired - open Claude Code".to_string()),
            );
        }
    }

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();

    let response = agent
        .get(USAGE_URL)
        .set("Authorization", &format!("Bearer {}", oauth.access_token))
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("User-Agent", "ai-usage-widget/0.1")
        .set("Accept", "application/json")
        .call();

    let response = match response {
        Ok(r) => r,
        Err(ureq::Error::Status(429, resp)) => {
            let retry_after = resp
                .header("retry-after")
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            return FetchOutcome::RateLimited { plan, retry_after };
        }
        Err(ureq::Error::Status(code, _)) => {
            let status = if code == 401 || code == 403 {
                ProviderStatus::Error("Token expired - open Claude Code".to_string())
            } else {
                ProviderStatus::Error(format!("HTTP {code}"))
            };
            return snap(plan, vec![], status);
        }
        Err(_) => {
            return snap(plan, vec![], ProviderStatus::Error("Network error".to_string()));
        }
    };

    let body = match response.into_string() {
        Ok(s) => s,
        Err(_) => {
            return snap(plan, vec![], ProviderStatus::Error("Network error".to_string()));
        }
    };

    match parse_usage(&body) {
        Ok(meters) => snap(plan, meters, ProviderStatus::Ok),
        Err(e) => snap(plan, vec![], ProviderStatus::Error(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_RESPONSE: &str = r#"{"five_hour":{"utilization":7.0,"resets_at":"2026-11-09T08:00:00.538378+00:00","limit_dollars":null},"seven_day":{"utilization":2.0,"resets_at":"2026-11-15T17:00:00.538396+00:00"},"seven_day_oauth_apps":null,"seven_day_opus":null,"seven_day_sonnet":null,"extra_usage":{"is_enabled":false,"monthly_limit":null,"used_credits":null,"utilization":null,"currency":null},"limits":[{"kind":"session","percent":7}],"spend":{"percent":0}}"#;

    #[test]
    fn parses_real_response() {
        let meters = parse_usage(REAL_RESPONSE).unwrap();
        assert_eq!(meters.len(), 2);
        assert_eq!(meters[0].label, "5h");
        assert!((meters[0].percent - 7.0).abs() < f32::EPSILON);
        let resets = meters[0].resets_at.unwrap();
        let expected_reset = DateTime::parse_from_rfc3339("2026-11-09T08:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(resets.timestamp(), expected_reset);

        assert_eq!(meters[1].label, "7d");
        assert!((meters[1].percent - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn parses_opus_sonnet_extra() {
        let json = r#"{
            "five_hour": {"utilization": 10.0, "resets_at": "2026-11-09T08:00:00Z"},
            "seven_day": {"utilization": 20.0, "resets_at": "2026-11-15T17:00:00Z"},
            "seven_day_opus": {"utilization": 30.0, "resets_at": null},
            "seven_day_sonnet": {"utilization": 40.0, "resets_at": null},
            "extra_usage": {"is_enabled": true, "utilization": 55.5}
        }"#;
        let meters = parse_usage(json).unwrap();
        assert_eq!(meters.len(), 5);
        assert_eq!(
            meters
                .iter()
                .map(|m| m.label.as_str())
                .collect::<Vec<_>>(),
            vec!["5h", "7d", "7d Opus", "7d Sonnet", "Extra"]
        );
        assert!((meters[4].percent - 55.5).abs() < f32::EPSILON);
    }

    #[test]
    fn clamps_percent() {
        let json = r#"{"five_hour":{"utilization":140.0,"resets_at":null}}"#;
        let meters = parse_usage(json).unwrap();
        assert_eq!(meters.len(), 1);
        assert!((meters[0].percent - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn invalid_json_errors() {
        assert_eq!(
            parse_usage("not json").unwrap_err(),
            "invalid usage JSON".to_string()
        );
    }

    #[test]
    fn plan_label_capitalizes() {
        assert_eq!(plan_label("team"), "Team");
        assert_eq!(plan_label("max"), "Max");
        assert_eq!(plan_label("pro"), "Pro");
        assert_eq!(plan_label(""), "");
    }
}
