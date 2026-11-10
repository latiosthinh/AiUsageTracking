use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Months, Utc};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::claude;
use crate::config::Config;
use crate::model::{Meter, ProviderSnapshot, ProviderStatus};

const USAGE_SUMMARY_URL: &str = "https://cursor.com/api/usage-summary";
const LEGACY_USAGE_URL: &str = "https://cursor.com/api/usage";
const PERIOD_USAGE_URL: &str =
    "https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage";
const PLAN_INFO_URL: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService/GetPlanInfo";

fn clamp_percent(p: f32) -> f32 {
    p.clamp(0.0, 100.0)
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_i64().map(|n| n as f64))
        .or_else(|| v.as_u64().map(|n| n as f64))
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn ms_to_utc(v: &Value) -> Option<DateTime<Utc>> {
    let ms = num(v)?;
    let secs = (ms / 1000.0).trunc() as i64;
    let subms = ms - (secs as f64 * 1000.0);
    let nsecs = (subms * 1_000_000.0).round() as u32;
    DateTime::from_timestamp(secs, nsecs)
}

fn parse_billing_cycle_end(v: &Value) -> Option<DateTime<Utc>> {
    v.get("billingCycleEnd")
        .and_then(|b| b.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

fn dollar_detail(used_cents: f64, limit_cents: f64) -> String {
    format!("${:.2} / ${:.2}", used_cents / 100.0, limit_cents / 100.0)
}

fn cursor_db_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Cursor").join("User").join("globalStorage").join("state.vscdb"))
}

fn read_db_row(key: &str) -> Option<String> {
    let path = cursor_db_path()?;
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let mut stmt = conn
        .prepare("SELECT value FROM ItemTable WHERE key = ?1")
        .ok()?;
    let mut rows = stmt.query(rusqlite::params![key]).ok()?;
    let row = rows.next().ok()??;
    let value: String = row.get(0).ok()?;
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub(crate) fn user_id_from_jwt(jwt: &str) -> Option<String> {
    let payload_b64 = jwt.split('.').nth(1)?;
    let mut payload_b64 = payload_b64.to_string();
    while payload_b64.ends_with('=') {
        payload_b64.pop();
    }
    let bytes = URL_SAFE_NO_PAD.decode(payload_b64.as_bytes()).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    let sub = v.get("sub")?.as_str()?;
    sub.rsplit('|').next().map(|s| s.to_string())
}

pub(crate) fn jwt_expired(jwt: &str, now_unix: i64) -> bool {
    let payload_b64 = match jwt.split('.').nth(1) {
        Some(p) => p,
        None => return false,
    };
    let mut payload_b64 = payload_b64.to_string();
    while payload_b64.ends_with('=') {
        payload_b64.pop();
    }
    let bytes = match URL_SAFE_NO_PAD.decode(payload_b64.as_bytes()) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let v: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return false,
    };
    match v.get("exp").and_then(|e| e.as_i64()) {
        Some(exp) => exp <= now_unix,
        None => false,
    }
}

fn jwt_from_cookie(cookie: &str) -> Option<&str> {
    if let Some((_, jwt)) = cookie.split_once("%3A%3A") {
        return Some(jwt);
    }
    cookie.split_once("::").map(|(_, jwt)| jwt)
}

fn normalize_config_token(token: &str) -> String {
    if token.contains("%3A%3A") {
        return token.to_string();
    }
    if let Some((user, jwt)) = token.split_once("::") {
        return format!("{user}%3A%3A{jwt}");
    }
    token.to_string()
}

fn session_cookie(cfg: &Config) -> Option<(String, Option<String>)> {
    if let Some(ref token) = cfg.cursor_session_token {
        let trimmed = token.trim();
        if !trimmed.is_empty() {
            return Some((normalize_config_token(trimmed), None));
        }
    }

    let jwt = read_db_row("cursorAuth/accessToken")?;
    let plan = read_db_row("cursorAuth/stripeMembershipType");
    let user_id = user_id_from_jwt(&jwt)?;
    let cookie = format!("{user_id}%3A%3A{jwt}");
    Some((cookie, plan))
}

pub fn parse_usage_summary(json: &str) -> Result<(Vec<Meter>, Option<String>), String> {
    let v: Value =
        serde_json::from_str(json).map_err(|_| "invalid usage JSON".to_string())?;

    let individual = v
        .get("individualUsage")
        .ok_or_else(|| "unexpected usage-summary shape".to_string())?;

    let resets_at = parse_billing_cycle_end(&v);

    let membership_type = v
        .get("membershipType")
        .and_then(|m| m.as_str())
        .map(|s| s.to_string());

    let mut meters = Vec::new();

    if let Some(plan_obj) = individual.get("plan") {
        if plan_obj.is_object() {
            let enabled = plan_obj
                .get("enabled")
                .and_then(|e| e.as_bool())
                .unwrap_or(true);
            if enabled {
                let used = plan_obj.get("used").and_then(|u| u.as_f64());
                let limit = plan_obj.get("limit").and_then(|l| l.as_f64());
                let percent = if let Some(p) = plan_obj.get("totalPercentUsed").and_then(|p| p.as_f64())
                {
                    p as f32
                } else if let (Some(u), Some(l)) = (used, limit) {
                    if l > 0.0 {
                        (u / l * 100.0) as f32
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };

                let detail = match (used, limit) {
                    (Some(u), Some(l)) if u.is_finite() && l.is_finite() => {
                        Some(dollar_detail(u, l))
                    }
                    _ => None,
                };

                meters.push(Meter {
                    label: "Plan".to_string(),
                    percent: clamp_percent(percent),
                    detail,
                    resets_at,
                });
            }
        }
    }

    if let Some(od) = individual.get("onDemand") {
        let enabled = od.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false);
        let limit = od.get("limit").and_then(|l| l.as_f64());
        if enabled {
            if let Some(lim) = limit {
                if lim > 0.0 {
                    let used = od.get("used").and_then(|u| u.as_f64()).unwrap_or(0.0);
                    let percent = (used / lim * 100.0) as f32;
                    let detail = if used.is_finite() && lim.is_finite() {
                        Some(dollar_detail(used, lim))
                    } else {
                        None
                    };
                    meters.push(Meter {
                        label: "On-demand".to_string(),
                        percent: clamp_percent(percent),
                        detail,
                        resets_at,
                    });
                }
            }
        }
    }

    Ok((meters, membership_type))
}

pub fn parse_period_usage(json: &str) -> Result<Vec<Meter>, String> {
    let v: Value =
        serde_json::from_str(json).map_err(|_| "invalid usage JSON".to_string())?;

    let plan_usage = v
        .get("planUsage")
        .filter(|p| p.is_object())
        .ok_or_else(|| "unexpected period-usage shape".to_string())?;

    let resets_at = v.get("billingCycleEnd").and_then(ms_to_utc);

    let auto = plan_usage
        .get("autoPercentUsed")
        .and_then(num)
        .unwrap_or(0.0) as f32;
    let api = plan_usage
        .get("apiPercentUsed")
        .and_then(num)
        .unwrap_or(0.0) as f32;
    let percent = plan_usage
        .get("totalPercentUsed")
        .and_then(num)
        .map(|p| p as f32)
        .unwrap_or(auto + api);

    let mut detail = format!("Auto {:.0}% · API {:.0}%", auto, api);
    if let Some(limit) = plan_usage.get("limit").and_then(num) {
        if limit > 0.0 {
            if let Some(remaining) = plan_usage.get("remaining").and_then(num) {
                detail.push_str(&format!(
                    " · ${:.2} / ${:.2}",
                    (limit - remaining) / 100.0,
                    limit / 100.0
                ));
            }
        }
    }

    let mut meters = vec![Meter {
        label: "Included".to_string(),
        percent: clamp_percent(percent),
        detail: Some(detail),
        resets_at,
    }];

    if let Some(slu) = v.get("spendLimitUsage") {
        let (limit, used) = if slu.get("individualLimit").and_then(num).is_some_and(|l| l > 0.0)
        {
            (
                slu.get("individualLimit").and_then(num).unwrap(),
                slu.get("individualUsed").and_then(num).unwrap_or(0.0),
            )
        } else if slu.get("pooledLimit").and_then(num).is_some_and(|l| l > 0.0) {
            (
                slu.get("pooledLimit").and_then(num).unwrap(),
                slu.get("pooledUsed").and_then(num).unwrap_or(0.0),
            )
        } else {
            (0.0, 0.0)
        };

        if limit > 0.0 {
            let pct = (used / limit * 100.0) as f32;
            meters.push(Meter {
                label: "On-demand".to_string(),
                percent: clamp_percent(pct),
                detail: Some(dollar_detail(used, limit)),
                resets_at,
            });
        }
    }

    Ok(meters)
}

pub fn parse_plan_info(json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json).ok()?;
    v.get("planInfo")
        .and_then(|p| p.get("planName"))
        .and_then(|n| n.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

pub fn parse_legacy_usage(json: &str) -> Result<Vec<Meter>, String> {
    let v: Value =
        serde_json::from_str(json).map_err(|_| "invalid usage JSON".to_string())?;

    let gpt4 = v
        .get("gpt-4")
        .ok_or_else(|| "unexpected usage-summary shape".to_string())?;

    let num_requests = gpt4
        .get("numRequests")
        .and_then(|n| n.as_u64())
        .or_else(|| gpt4.get("numRequests").and_then(|n| n.as_i64()).map(|n| n as u64))
        .unwrap_or(0);

    let max_request = gpt4.get("maxRequestUsage").and_then(|m| {
        m.as_u64()
            .or_else(|| m.as_i64().map(|n| n as u64))
    });

    let percent = match max_request {
        Some(max) if max > 0 => (num_requests as f32 / max as f32) * 100.0,
        _ => 0.0,
    };

    let detail = match max_request {
        Some(max) if max > 0 => Some(format!("{num_requests} / {max}")),
        _ => Some(num_requests.to_string()),
    };

    let resets_at = v
        .get("startOfMonth")
        .and_then(|s| s.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .and_then(|start| start.checked_add_months(Months::new(1)));

    Ok(vec![Meter {
        label: "Requests".to_string(),
        percent: clamp_percent(percent),
        detail,
        resets_at,
    }])
}

fn apply_cursor_headers(req: ureq::Request, cookie: &str) -> ureq::Request {
    req.set("Cookie", &format!("WorkosCursorSessionToken={cookie}"))
        .set("User-Agent", "ai-usage-widget/0.1")
        .set("Accept", "application/json")
        .set("Origin", "https://cursor.com")
        .set("Referer", "https://cursor.com/dashboard")
}

fn api2_post(
    agent: &ureq::Agent,
    url: &str,
    jwt: &str,
) -> Result<ureq::Response, Box<ureq::Error>> {
    agent
        .post(url)
        .set("Authorization", &format!("Bearer {jwt}"))
        .set("Content-Type", "application/json")
        .set("Connect-Protocol-Version", "1")
        .set("User-Agent", "ai-usage-widget/0.1")
        .send_string("{}")
        .map_err(Box::new)
}

fn fetch_plan_name(agent: &ureq::Agent, jwt: &str) -> Option<String> {
    let resp = api2_post(agent, PLAN_INFO_URL, jwt).ok()?;
    if resp.status() != 200 {
        return None;
    }
    let body = resp.into_string().ok()?;
    parse_plan_info(&body)
}

fn cursor_plan_label(
    api_plan: Option<String>,
    membership: Option<String>,
    db_plan: Option<String>,
) -> Option<String> {
    let plan_raw = api_plan.or(membership).or(db_plan);
    plan_raw
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(claude::plan_label)
}

pub fn fetch(cfg: &Config) -> ProviderSnapshot {
    let fetched_at = Utc::now();

    let (cookie, db_plan) = match session_cookie(cfg) {
        Some(pair) => pair,
        None => {
            return ProviderSnapshot {
                name: "Cursor",
                plan: None,
                meters: vec![],
                status: ProviderStatus::NotSignedIn("Not signed in".to_string()),
                fetched_at,
            };
        }
    };

    if let Some(jwt) = jwt_from_cookie(&cookie) {
        let now_unix = Utc::now().timestamp();
        if jwt_expired(jwt, now_unix) {
            return ProviderSnapshot {
                name: "Cursor",
                plan: None,
                meters: vec![],
                status: ProviderStatus::Error("Session expired - sign in to Cursor".to_string()),
                fetched_at,
            };
        }
    }

    let user_id = jwt_from_cookie(&cookie)
        .and_then(user_id_from_jwt)
        .or_else(|| cookie.split_once("%3A%3A").map(|(u, _)| u.to_string()));

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();

    if let Some(jwt) = jwt_from_cookie(&cookie) {
        match api2_post(&agent, PERIOD_USAGE_URL, jwt) {
            Ok(resp) if resp.status() == 200 => {
                if let Ok(body) = resp.into_string() {
                    if let Ok(meters) = parse_period_usage(&body) {
                        let api_plan = fetch_plan_name(&agent, jwt);
                        let plan = cursor_plan_label(api_plan, None, db_plan.clone());
                        return ProviderSnapshot {
                            name: "Cursor",
                            plan,
                            meters,
                            status: ProviderStatus::Ok,
                            fetched_at,
                        };
                    }
                }
            }
            Ok(resp) => {
                let code = resp.status();
                if code == 401 || code == 403 {
                    return ProviderSnapshot {
                        name: "Cursor",
                        plan: None,
                        meters: vec![],
                        status: ProviderStatus::Error(
                            "Session expired - sign in to Cursor".to_string(),
                        ),
                        fetched_at,
                    };
                }
            }
            Err(e) if matches!(&*e, ureq::Error::Status(code, _) if *code == 401 || *code == 403) => {
                return ProviderSnapshot {
                    name: "Cursor",
                    plan: None,
                    meters: vec![],
                    status: ProviderStatus::Error(
                        "Session expired - sign in to Cursor".to_string(),
                    ),
                    fetched_at,
                };
            }
            _ => {}
        }
    }

    let summary_resp = apply_cursor_headers(agent.get(USAGE_SUMMARY_URL), &cookie).call();

    match summary_resp {
        Ok(resp) if resp.status() == 200 => {
            let body = match resp.into_string() {
                Ok(s) => s,
                Err(_) => {
                    return ProviderSnapshot {
                        name: "Cursor",
                        plan: None,
                        meters: vec![],
                        status: ProviderStatus::Error("Network error".to_string()),
                        fetched_at,
                    };
                }
            };
            match parse_usage_summary(&body) {
                Ok((meters, membership)) => {
                    let plan = cursor_plan_label(None, membership, db_plan.clone());
                    return ProviderSnapshot {
                        name: "Cursor",
                        plan,
                        meters,
                        status: ProviderStatus::Ok,
                        fetched_at,
                    };
                }
                Err(_) => {
                    // fall through to legacy
                }
            }
        }
        Ok(resp) => {
            let code = resp.status();
            if code == 401 || code == 403 {
                return ProviderSnapshot {
                    name: "Cursor",
                    plan: None,
                    meters: vec![],
                    status: ProviderStatus::Error("Session expired - sign in to Cursor".to_string()),
                    fetched_at,
                };
            }
            if code != 404 {
                return ProviderSnapshot {
                    name: "Cursor",
                    plan: db_plan.as_deref().filter(|s| !s.is_empty()).map(claude::plan_label),
                    meters: vec![],
                    status: ProviderStatus::Error(format!("HTTP {code}")),
                    fetched_at,
                };
            }
        }
        Err(ureq::Error::Status(code, _)) => {
            if code == 401 || code == 403 {
                return ProviderSnapshot {
                    name: "Cursor",
                    plan: None,
                    meters: vec![],
                    status: ProviderStatus::Error("Session expired - sign in to Cursor".to_string()),
                    fetched_at,
                };
            }
            if code != 404 {
                return ProviderSnapshot {
                    name: "Cursor",
                    plan: db_plan.as_deref().filter(|s| !s.is_empty()).map(claude::plan_label),
                    meters: vec![],
                    status: ProviderStatus::Error(format!("HTTP {code}")),
                    fetched_at,
                };
            }
        }
        Err(_) => {
            return ProviderSnapshot {
                name: "Cursor",
                plan: None,
                meters: vec![],
                status: ProviderStatus::Error("Network error".to_string()),
                fetched_at,
            };
        }
    }

    let Some(user_id) = user_id else {
        return ProviderSnapshot {
            name: "Cursor",
            plan: None,
            meters: vec![],
            status: ProviderStatus::Error("Session expired - sign in to Cursor".to_string()),
            fetched_at,
        };
    };

    let legacy_url = format!("{LEGACY_USAGE_URL}?user={user_id}");
    let legacy_resp = apply_cursor_headers(agent.get(&legacy_url), &cookie).call();

    match legacy_resp {
        Ok(resp) if resp.status() == 200 => {
            let body = match resp.into_string() {
                Ok(s) => s,
                Err(_) => {
                    return ProviderSnapshot {
                        name: "Cursor",
                        plan: None,
                        meters: vec![],
                        status: ProviderStatus::Error("Network error".to_string()),
                        fetched_at,
                    };
                }
            };
            match parse_legacy_usage(&body) {
                Ok(meters) => ProviderSnapshot {
                    name: "Cursor",
                    plan: db_plan.as_deref().filter(|s| !s.is_empty()).map(claude::plan_label),
                    meters,
                    status: ProviderStatus::Ok,
                    fetched_at,
                },
                Err(e) => ProviderSnapshot {
                    name: "Cursor",
                    plan: db_plan.as_deref().filter(|s| !s.is_empty()).map(claude::plan_label),
                    meters: vec![],
                    status: ProviderStatus::Error(e),
                    fetched_at,
                },
            }
        }
        Ok(resp) => {
            let code = resp.status();
            let status = if code == 401 || code == 403 {
                ProviderStatus::Error("Session expired - sign in to Cursor".to_string())
            } else {
                ProviderStatus::Error(format!("HTTP {code}"))
            };
            ProviderSnapshot {
                name: "Cursor",
                plan: db_plan.as_deref().filter(|s| !s.is_empty()).map(claude::plan_label),
                meters: vec![],
                status,
                fetched_at,
            }
        }
        Err(ureq::Error::Status(code, _)) => {
            let status = if code == 401 || code == 403 {
                ProviderStatus::Error("Session expired - sign in to Cursor".to_string())
            } else {
                ProviderStatus::Error(format!("HTTP {code}"))
            };
            ProviderSnapshot {
                name: "Cursor",
                plan: db_plan.as_deref().filter(|s| !s.is_empty()).map(claude::plan_label),
                meters: vec![],
                status,
                fetched_at,
            }
        }
        Err(_) => ProviderSnapshot {
            name: "Cursor",
            plan: None,
            meters: vec![],
            status: ProviderStatus::Error("Network error".to_string()),
            fetched_at,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USAGE_SUMMARY_FIXTURE: &str = r#"{"billingCycleStart":"2026-10-20T00:00:00.000Z","billingCycleEnd":"2026-11-20T00:00:00.000Z",
     "membershipType":"pro","limitType":"user","isUnlimited":false,
     "individualUsage":{
       "plan":{"enabled":true,"used":1520,"limit":2000,"remaining":480,"totalPercentUsed":76.0,
               "breakdown":{"included":1520,"bonus":0,"total":1520}},
       "onDemand":{"enabled":true,"used":250,"limit":5000,"remaining":4750}},
     "teamUsage":{"onDemand":{"enabled":true,"used":0,"limit":null,"remaining":null}}}"#;

    #[test]
    fn parses_usage_summary() {
        let (meters, membership) = parse_usage_summary(USAGE_SUMMARY_FIXTURE).unwrap();
        assert_eq!(membership.as_deref(), Some("pro"));
        assert_eq!(meters.len(), 2);

        let plan = &meters[0];
        assert_eq!(plan.label, "Plan");
        assert!((plan.percent - 76.0).abs() < f32::EPSILON);
        assert_eq!(plan.detail.as_deref(), Some("$15.20 / $20.00"));
        let reset = plan.resets_at.unwrap();
        assert_eq!(
            reset,
            DateTime::parse_from_rfc3339("2026-11-20T00:00:00.000Z")
                .unwrap()
                .with_timezone(&Utc)
        );

        let od = &meters[1];
        assert_eq!(od.label, "On-demand");
        assert!((od.percent - 5.0).abs() < f32::EPSILON);
        assert_eq!(od.detail.as_deref(), Some("$2.50 / $50.00"));
    }

    #[test]
    fn usage_summary_percent_fallback() {
        let json = r#"{"billingCycleEnd":"2026-11-20T00:00:00.000Z",
        "individualUsage":{"plan":{"enabled":true,"used":1520,"limit":2000},"onDemand":{"enabled":false}}}"#;
        let (meters, _) = parse_usage_summary(json).unwrap();
        assert_eq!(meters.len(), 1);
        assert!((meters[0].percent - 76.0).abs() < 0.1);
    }

    #[test]
    fn on_demand_disabled_skipped() {
        let json = r#"{"billingCycleEnd":"2026-11-20T00:00:00.000Z",
        "individualUsage":{"plan":{"enabled":true,"used":100,"limit":200,"totalPercentUsed":50.0},
        "onDemand":{"enabled":false,"used":250,"limit":5000}}}"#;
        let (meters, _) = parse_usage_summary(json).unwrap();
        assert_eq!(meters.len(), 1);
        assert_eq!(meters[0].label, "Plan");
    }

    const LEGACY_FIXTURE: &str = r#"{"gpt-4":{"numRequests":123,"numRequestsTotal":130,"numTokens":456789,"maxRequestUsage":500,"maxTokenUsage":null},
     "gpt-3.5-turbo":{"numRequests":5,"numRequestsTotal":5,"numTokens":100,"maxRequestUsage":null,"maxTokenUsage":null},
     "startOfMonth":"2026-10-20T08:00:00.000Z"}"#;

    #[test]
    fn parses_legacy_usage() {
        let meters = parse_legacy_usage(LEGACY_FIXTURE).unwrap();
        assert_eq!(meters.len(), 1);
        assert!((meters[0].percent - 24.6).abs() < 0.1);
        assert_eq!(meters[0].detail.as_deref(), Some("123 / 500"));
        let reset = meters[0].resets_at.unwrap();
        assert_eq!(
            reset,
            DateTime::parse_from_rfc3339("2026-11-20T08:00:00.000Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn user_id_from_jwt_works() {
        let payload = r#"{"sub":"auth0|user_01ABC","exp":4102444800}"#;
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let jwt = format!("header.{payload_b64}.sig");
        assert_eq!(user_id_from_jwt(&jwt).as_deref(), Some("user_01ABC"));
    }

    #[test]
    fn jwt_expired_works() {
        let payload = r#"{"sub":"x","exp":1000}"#;
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let jwt = format!("h.{payload_b64}.s");
        assert!(jwt_expired(&jwt, 2000));
        assert!(!jwt_expired(&jwt, 500));
    }

    #[test]
    fn invalid_json_errors() {
        assert_eq!(
            parse_usage_summary("not json").unwrap_err(),
            "invalid usage JSON".to_string()
        );
    }

    const PERIOD_USAGE_REAL: &str = r#"{"billingCycleStart":"1793773441307","billingCycleEnd":"1794378241307",
     "planUsage":{"remainingBonus":false,"bonusTooltip":"...","autoPercentUsed":5.742145178764897,"totalPercentUsed":5.742145178764897},
     "spendLimitUsage":{"pooledLimit":"0","pooledUsed":0,"pooledRemaining":"0","limitType":"team"},
     "displayThreshold":200,"enabled":true,"displayMessage":"You've used 6% of your included usage"}"#;

    #[test]
    fn parses_period_usage_real() {
        let meters = parse_period_usage(PERIOD_USAGE_REAL).unwrap();
        assert_eq!(meters.len(), 1);
        let inc = &meters[0];
        assert_eq!(inc.label, "Included");
        assert!((inc.percent - 5.742).abs() < 0.01);
        assert_eq!(inc.detail.as_deref(), Some("Auto 6% · API 0%"));
        let reset = inc.resets_at.unwrap();
        assert_eq!(reset.timestamp(), 1_794_378_241);
    }

    #[test]
    fn period_usage_with_limit_and_on_demand() {
        let json = r#"{"billingCycleEnd":1794378241307,
        "planUsage":{"totalPercentUsed":40,"autoPercentUsed":30,"apiPercentUsed":10,"limit":2000,"remaining":1200},
        "spendLimitUsage":{"individualLimit":"5000","individualUsed":1250}}"#;
        let meters = parse_period_usage(json).unwrap();
        assert_eq!(meters.len(), 2);
        assert_eq!(meters[0].detail.as_deref(), Some("Auto 30% · API 10% · $8.00 / $20.00"));
        assert!((meters[0].percent - 40.0).abs() < f32::EPSILON);
        assert_eq!(meters[1].label, "On-demand");
        assert!((meters[1].percent - 25.0).abs() < f32::EPSILON);
        assert_eq!(meters[1].detail.as_deref(), Some("$12.50 / $50.00"));
    }

    #[test]
    fn period_usage_missing_plan_usage_errors() {
        assert_eq!(
            parse_period_usage(r#"{"billingCycleEnd":"1"}"#).unwrap_err(),
            "unexpected period-usage shape".to_string()
        );
    }

    const PLAN_INFO_FIXTURE: &str =
        r#"{"planInfo":{"planName":"Premium","price":"","billingCycleEnd":"1796083200000","includedUsagePeriod":"INCLUDED_USAGE_PERIOD_WEEKLY","includedUsageResetsAt":"1794378241307"},"isMerged":true}"#;

    #[test]
    fn parses_plan_info() {
        assert_eq!(parse_plan_info(PLAN_INFO_FIXTURE).as_deref(), Some("Premium"));
        assert_eq!(parse_plan_info("{}"), None);
    }
}
