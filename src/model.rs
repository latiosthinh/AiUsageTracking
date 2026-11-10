use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum Severity {
    Normal,
    Warning,
    Critical,
}

#[allow(dead_code)]
impl Severity {
    /// < 70.0 => Normal, < 90.0 => Warning, else Critical
    pub fn from_percent(p: f32) -> Severity {
        if p < 70.0 {
            Severity::Normal
        } else if p < 90.0 {
            Severity::Warning
        } else {
            Severity::Critical
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Meter {
    pub label: String,
    pub percent: f32,
    pub detail: Option<String>,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderStatus {
    Ok,
    NotSignedIn(String),
    Error(String),
    /// Meters are from an earlier successful fetch; the message says why they're not fresh.
    Stale(String),
}

impl ProviderStatus {
    /// True when the snapshot's meters should be drawn.
    pub fn has_meters(&self) -> bool {
        matches!(self, ProviderStatus::Ok | ProviderStatus::Stale(_))
    }
}

#[derive(Debug, Clone)]
pub struct ProviderSnapshot {
    #[allow(dead_code)]
    pub name: &'static str,
    pub plan: Option<String>,
    pub meters: Vec<Meter>,
    pub status: ProviderStatus,
    pub fetched_at: DateTime<Utc>,
}

/// Human countdown, e.g. "4d 3h", "2h 05m", "12m", "now" (if <= 0).
/// Rules: >= 1 day -> "{d}d {h}h"; >= 1 hour -> "{h}h {mm}m" (minutes zero-padded to 2); else "{m}m"; <= 0 -> "now".
pub fn format_countdown(target: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let diff = target.signed_duration_since(now);
    let total_secs = diff.num_seconds();
    if total_secs <= 0 {
        return "now".to_string();
    }

    let days = total_secs / 86_400;
    let hours = (total_secs % 86_400) / 3_600;
    let minutes = (total_secs % 3_600) / 60;

    if days >= 1 {
        format!("{days}d {hours}h")
    } else if hours >= 1 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_thresholds() {
        assert_eq!(Severity::from_percent(69.9), Severity::Normal);
        assert_eq!(Severity::from_percent(70.0), Severity::Warning);
        assert_eq!(Severity::from_percent(89.9), Severity::Warning);
        assert_eq!(Severity::from_percent(90.0), Severity::Critical);
    }

    #[test]
    fn countdown_formats() {
        let base = DateTime::parse_from_rfc3339("2026-01-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let now = base;
        let target = base;
        assert_eq!(format_countdown(target, now), "now");

        let target = now + chrono::Duration::minutes(12);
        assert_eq!(format_countdown(target, now), "12m");

        let target = now + chrono::Duration::hours(2) + chrono::Duration::minutes(5);
        assert_eq!(format_countdown(target, now), "2h 05m");

        let target = now + chrono::Duration::days(4) + chrono::Duration::hours(3);
        assert_eq!(format_countdown(target, now), "4d 3h");
    }
}
