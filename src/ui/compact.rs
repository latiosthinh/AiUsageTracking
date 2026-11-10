use chrono::{DateTime, Local, TimeZone, Utc};
use eframe::egui::{self, Align2, Color32, Rect, Sense, Shape, Stroke};
use egui::FontId;

use crate::model::{format_countdown, Meter, ProviderSnapshot, ProviderStatus};

pub const ROW_HEIGHT: f32 = 16.0;
pub const RINGS_HEIGHT: f32 = 22.0;
pub const MAX_METERS: usize = 3;

fn body_font() -> FontId {
    FontId::proportional(11.0)
}

/// Local reset time for the compact strip: "HH:MM" within 24 h, "Ddd HH:MM" beyond, "now" if past.
pub fn reset_time_label(resets_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    reset_time_label_in(resets_at, now, &Local)
}

fn reset_time_label_in<Tz: TimeZone>(resets_at: DateTime<Utc>, now: DateTime<Utc>, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if resets_at <= now {
        return "now".to_string();
    }
    let reset_local = resets_at.with_timezone(tz);
    let diff = resets_at - now;
    if diff < chrono::Duration::hours(24) {
        reset_local.format("%H:%M").to_string()
    } else {
        reset_local.format("%a %H:%M").to_string()
    }
}

/// Short label for the compact strip.
pub fn compact_label(label: &str) -> String {
    match label {
        "5h" => "5h".to_string(),
        "7d" => "7d".to_string(),
        "7d Opus" => "Op".to_string(),
        "7d Sonnet" => "So".to_string(),
        "Extra" => "Ex".to_string(),
        "Plan" => "Plan".to_string(),
        "Included" => "Inc".to_string(),
        "On-demand" => "OD".to_string(),
        "Requests" => "Req".to_string(),
        _ => label.chars().take(4).collect(),
    }
}

fn truncate_status_msg(msg: &str) -> String {
    let char_count = msg.chars().count();
    if char_count <= 28 {
        msg.to_string()
    } else {
        format!("{}…", msg.chars().take(28).collect::<String>())
    }
}

fn paint_name_cell(ui: &mut egui::Ui, name: &str) {
    let font = body_font();
    let (r, _) = ui.allocate_exact_size(egui::vec2(42.0, ROW_HEIGHT), Sense::hover());
    ui.painter().text(
        r.left_center(),
        Align2::LEFT_CENTER,
        name,
        font,
        ui.visuals().strong_text_color(),
    );
}

fn paint_text_cell(ui: &mut egui::Ui, text: &str, color: Color32, hover: Option<&str>) -> Rect {
    let font = body_font();
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), color);
    let size = egui::vec2(galley.size().x, ROW_HEIGHT);
    let (r, response) = ui.allocate_exact_size(size, Sense::hover());
    let pos = egui::pos2(r.left(), r.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, color);
    if let Some(tip) = hover {
        response.on_hover_text(tip);
    }
    r
}

fn paint_meter_compact(ui: &mut egui::Ui, m: &Meter, now: DateTime<Utc>) {
    let font = body_font();
    let gray = Color32::GRAY;
    let text_color = ui.visuals().text_color();

    let lead = m
        .resets_at
        .map(|r| reset_time_label(r, now))
        .unwrap_or_else(|| compact_label(&m.label));
    let galley = ui
        .painter()
        .layout_no_wrap(lead, font.clone(), gray);
    let (r, _) = ui.allocate_exact_size(
        egui::vec2(galley.size().x, ROW_HEIGHT),
        Sense::hover(),
    );
    let pos = egui::pos2(r.left(), r.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, gray);

    let (rect, response) = ui.allocate_exact_size(egui::vec2(36.0, ROW_HEIGHT), Sense::hover());
    let bar_rect = Rect::from_center_size(rect.center(), egui::vec2(36.0, 5.0));
    super::paint_meter_bar(ui.painter(), bar_rect, m.percent);

    let mut hover = format!("{}: {:.0}%", m.label, m.percent);
    if let Some(resets) = m.resets_at {
        hover.push_str(&format!(" · resets in {}", format_countdown(resets, now)));
    }
    if let Some(ref detail) = m.detail {
        hover.push('\n');
        hover.push_str(detail);
    }
    response.on_hover_text(hover);

    let (r, _) = ui.allocate_exact_size(egui::vec2(30.0, ROW_HEIGHT), Sense::hover());
    ui.painter().text(
        r.right_center(),
        Align2::RIGHT_CENTER,
        format!("{:.0}%", m.percent),
        font,
        text_color,
    );
}

fn draw_compact_row(
    ui: &mut egui::Ui,
    default_name: &str,
    snapshot: Option<&ProviderSnapshot>,
    now: DateTime<Utc>,
) -> Rect {
    let row_w = ui.available_width().max(1.0);
    let (row_rect, _) = ui.allocate_exact_size(egui::vec2(row_w, ROW_HEIGHT), Sense::hover());
    let mut row_ui = ui.new_child(egui::UiBuilder::new().max_rect(row_rect));
    row_ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;

        let display_name = snapshot.map(|s| s.name).unwrap_or(default_name);
        paint_name_cell(ui, display_name);

        match snapshot {
            None => {
                paint_text_cell(ui, "loading…", Color32::GRAY, None);
            }
            Some(s) => match &s.status {
                ProviderStatus::NotSignedIn(msg) => {
                    let shown = truncate_status_msg(msg);
                    paint_text_cell(ui, &shown, Color32::GRAY, Some(msg.as_str()));
                }
                ProviderStatus::Error(msg) => {
                    let shown = truncate_status_msg(msg);
                    let error_red = Color32::from_rgb(229, 57, 53);
                    paint_text_cell(ui, &shown, error_red, Some(msg.as_str()));
                }
                ProviderStatus::Ok => {
                    for m in s.meters.iter().take(MAX_METERS) {
                        paint_meter_compact(ui, m, now);
                    }
                }
                ProviderStatus::Stale(msg) => {
                    for m in s.meters.iter().take(MAX_METERS) {
                        paint_meter_compact(ui, m, now);
                    }
                    let amber = Color32::from_rgb(255, 179, 0);
                    paint_text_cell(ui, "⚠", amber, Some(msg.as_str()));
                }
            },
        }
    });
    Rect::from_min_size(
        row_rect.min,
        egui::vec2(row_ui.min_rect().width(), ROW_HEIGHT),
    )
}

/// Draws the two rows top-to-bottom and returns the union rect and whether refresh was clicked.
pub fn draw_compact(
    ui: &mut egui::Ui,
    claude: Option<&ProviderSnapshot>,
    cursor: Option<&ProviderSnapshot>,
    now: DateTime<Utc>,
    refreshing: bool,
) -> (egui::Rect, bool) {
    let mut union = Rect::NOTHING;
    let mut second_row_min_y = 0.0_f32;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        union = union.union(draw_compact_row(ui, "Claude", claude, now));
        let second = draw_compact_row(ui, "Cursor", cursor, now);
        second_row_min_y = second.min.y;
        union = union.union(second);
    });

    let refresh_rect = Rect::from_min_size(
        egui::pos2(union.max.x + 4.0, second_row_min_y),
        egui::vec2(12.0, ROW_HEIGHT),
    );
    let sense = if refreshing {
        Sense::hover()
    } else {
        Sense::click()
    };
    let response = ui.interact(refresh_rect, egui::Id::new("compact_refresh"), sense);
    let color = if refreshing {
        Color32::from_gray(80)
    } else if response.hovered() {
        ui.visuals().strong_text_color()
    } else {
        Color32::GRAY
    };
    let refresh_clicked = if refreshing {
        false
    } else {
        response.clicked()
    };
    if !refreshing && response.hovered() {
        response.on_hover_text("Refresh now");
    }
    ui.painter().text(
        refresh_rect.center(),
        Align2::CENTER_CENTER,
        "⟳",
        body_font(),
        color,
    );

    union = union.union(refresh_rect);
    (union, refresh_clicked)
}

pub(crate) fn arc_points(
    center: egui::Pos2,
    radius: f32,
    fraction: f32,
    segments: usize,
) -> Vec<egui::Pos2> {
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction <= 0.0 {
        return Vec::new();
    }
    let n = 2_usize.max((segments as f32 * fraction).ceil() as usize) + 1;
    let start = -std::f32::consts::FRAC_PI_2;
    let sweep = fraction * std::f32::consts::TAU;
    (0..n)
        .map(|i| {
            let t = i as f32 / (n - 1) as f32;
            let angle = start + t * sweep;
            egui::pos2(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
        .collect()
}

fn paint_claude_icon(ui: &mut egui::Ui) -> egui::Response {
    let (slot, response) =
        ui.allocate_exact_size(egui::vec2(16.0, RINGS_HEIGHT), Sense::hover());
    let cell = Rect::from_center_size(slot.center(), egui::vec2(16.0, 16.0));
    let center = cell.center();
    let color = Color32::from_rgb(217, 119, 87);
    let stroke = Stroke::new(2.0, color);
    let painter = ui.painter();
    for deg in [0.0_f32, 45.0, 90.0, 135.0] {
        let rad = deg.to_radians();
        let (cx, sy) = (rad.cos(), rad.sin());
        let half = 6.0;
        painter.line_segment(
            [
                egui::pos2(center.x - half * cx, center.y - half * sy),
                egui::pos2(center.x + half * cx, center.y + half * sy),
            ],
            stroke,
        );
    }
    response
}

fn paint_cursor_icon(ui: &mut egui::Ui) -> egui::Response {
    let (slot, response) =
        ui.allocate_exact_size(egui::vec2(16.0, RINGS_HEIGHT), Sense::hover());
    let cell = Rect::from_center_size(slot.center(), egui::vec2(16.0, 16.0));
    let m = cell.min;
    let color = Color32::from_gray(210);
    let painter = ui.painter();
    painter.add(Shape::convex_polygon(
        vec![
            egui::pos2(m.x + 4.0, m.y + 2.0),
            egui::pos2(m.x + 4.0, m.y + 14.0),
            egui::pos2(m.x + 13.0, m.y + 10.0),
        ],
        color,
        Stroke::NONE,
    ));
    painter.line_segment(
        [
            egui::pos2(m.x + 8.0, m.y + 11.0),
            egui::pos2(m.x + 11.0, m.y + 15.0),
        ],
        Stroke::new(2.0, color),
    );
    response
}

fn paint_ring(ui: &mut egui::Ui, provider: &str, m: &Meter, now: DateTime<Utc>) -> Rect {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(22.0, RINGS_HEIGHT), Sense::hover());
    let center = rect.center();
    let track = Color32::from_gray(60);
    let color = super::severity_color(crate::model::Severity::from_percent(m.percent));
    let painter = ui.painter();
    painter.circle_stroke(center, 8.5, Stroke::new(3.0, track));
    if m.percent > 0.0 {
        let points = arc_points(center, 8.5, m.percent / 100.0, 48);
        if !points.is_empty() {
            painter.add(Shape::line(points, Stroke::new(3.0, color)));
        }
    }

    let mut hover = format!("{} · {}: {:.0}%", provider, m.label, m.percent);
    if let Some(resets) = m.resets_at {
        hover.push_str(&format!(
            " · resets {} (in {})",
            reset_time_label(resets, now),
            format_countdown(resets, now)
        ));
    }
    if let Some(ref detail) = m.detail {
        hover.push('\n');
        hover.push_str(detail);
    }
    response.on_hover_text(hover);
    rect
}

fn provider_icon_hover(provider: &str, snapshot: &ProviderSnapshot) -> String {
    let mut tip = provider.to_string();
    if let Some(ref plan) = snapshot.plan {
        tip.push_str(&format!(" · {}", plan));
    }
    match &snapshot.status {
        ProviderStatus::Ok => {}
        ProviderStatus::NotSignedIn(msg) | ProviderStatus::Error(msg) | ProviderStatus::Stale(msg) => {
            tip.push('\n');
            tip.push_str(msg);
        }
    }
    tip
}

fn draw_rings_provider(
    ui: &mut egui::Ui,
    provider: &str,
    snapshot: Option<&ProviderSnapshot>,
    now: DateTime<Utc>,
    paint_icon: fn(&mut egui::Ui) -> egui::Response,
) -> Rect {
    let icon_resp = paint_icon(ui);
    let mut union = icon_resp.rect;

    match snapshot {
        None => {
            union = union.union(paint_text_cell(ui, "…", Color32::GRAY, Some("Loading")));
        }
        Some(s) => {
            icon_resp.on_hover_text(provider_icon_hover(provider, s));
            match &s.status {
                ProviderStatus::NotSignedIn(msg) => {
                    union = union.union(paint_text_cell(ui, "⚠", Color32::GRAY, Some(msg.as_str())));
                }
                ProviderStatus::Error(msg) => {
                    let error_red = Color32::from_rgb(229, 57, 53);
                    union =
                        union.union(paint_text_cell(ui, "⚠", error_red, Some(msg.as_str())));
                }
                ProviderStatus::Ok => {
                    ui.add_space(4.0);
                    for (i, m) in s.meters.iter().take(MAX_METERS).enumerate() {
                        if i > 0 {
                            ui.add_space(3.0);
                        }
                        union = union.union(paint_ring(ui, provider, m, now));
                    }
                }
                ProviderStatus::Stale(msg) => {
                    ui.add_space(4.0);
                    for (i, m) in s.meters.iter().take(MAX_METERS).enumerate() {
                        if i > 0 {
                            ui.add_space(3.0);
                        }
                        union = union.union(paint_ring(ui, provider, m, now));
                    }
                    let amber = Color32::from_rgb(255, 179, 0);
                    union = union.union(paint_text_cell(ui, "⚠", amber, Some(msg.as_str())));
                }
            }
        }
    }

    union
}

/// One-row strip: [Claude icon][rings…]  [Cursor icon][rings…]  [⟳].
/// Returns the union rect of everything drawn and whether ⟳ was clicked.
pub fn draw_rings(
    ui: &mut egui::Ui,
    claude: Option<&ProviderSnapshot>,
    cursor: Option<&ProviderSnapshot>,
    now: DateTime<Utc>,
    refreshing: bool,
) -> (egui::Rect, bool) {
    let mut union = Rect::NOTHING;
    let mut refresh_clicked = false;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;

        union = union.union(draw_rings_provider(ui, "Claude", claude, now, paint_claude_icon));

        ui.add_space(10.0);

        union = union.union(draw_rings_provider(ui, "Cursor", cursor, now, paint_cursor_icon));

        ui.add_space(6.0);

        let (refresh_rect, _) =
            ui.allocate_exact_size(egui::vec2(12.0, RINGS_HEIGHT), Sense::hover());
        let sense = if refreshing {
            Sense::hover()
        } else {
            Sense::click()
        };
        let response = ui.interact(refresh_rect, egui::Id::new("rings_refresh"), sense);
        let color = if refreshing {
            Color32::from_gray(80)
        } else if response.hovered() {
            ui.visuals().strong_text_color()
        } else {
            Color32::GRAY
        };
        refresh_clicked = if refreshing { false } else { response.clicked() };
        if !refreshing && response.hovered() {
            response.on_hover_text("Refresh now");
        }
        ui.painter().text(
            refresh_rect.center(),
            Align2::CENTER_CENTER,
            "⟳",
            body_font(),
            color,
        );
        union = union.union(refresh_rect);
    });

    (union, refresh_clicked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Utc};
    use egui::RawInput;

    fn tz_plus7() -> FixedOffset {
        FixedOffset::east_opt(7 * 3600).unwrap()
    }

    fn t(s: &str) -> DateTime<Utc> {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ")
            .unwrap()
            .and_utc()
    }

    #[test]
    fn reset_label_same_day() {
        let tz = tz_plus7();
        let now = t("2026-11-09T03:00:00Z");
        assert_eq!(
            reset_time_label_in(t("2026-11-09T07:30:00Z"), now, &tz),
            "14:30"
        );
    }

    #[test]
    fn reset_label_under_24h_next_day() {
        let tz = tz_plus7();
        let now = t("2026-11-09T03:00:00Z");
        assert_eq!(
            reset_time_label_in(t("2026-11-10T01:00:00Z"), now, &tz),
            "08:00"
        );
    }

    #[test]
    fn reset_label_beyond_24h() {
        let tz = tz_plus7();
        let now = t("2026-11-09T03:00:00Z");
        assert_eq!(
            reset_time_label_in(t("2026-11-13T02:00:00Z"), now, &tz),
            "Fri 09:00"
        );
    }

    #[test]
    fn reset_label_past_is_now() {
        let tz = tz_plus7();
        let now = t("2026-11-09T03:00:00Z");
        assert_eq!(
            reset_time_label_in(t("2026-11-09T02:00:00Z"), now, &tz),
            "now"
        );
    }

    #[test]
    fn compact_labels_known() {
        assert_eq!(compact_label("5h"), "5h");
        assert_eq!(compact_label("7d"), "7d");
        assert_eq!(compact_label("7d Opus"), "Op");
        assert_eq!(compact_label("7d Sonnet"), "So");
        assert_eq!(compact_label("Extra"), "Ex");
        assert_eq!(compact_label("Plan"), "Plan");
        assert_eq!(compact_label("Included"), "Inc");
        assert_eq!(compact_label("On-demand"), "OD");
        assert_eq!(compact_label("Requests"), "Req");
    }

    #[test]
    fn compact_label_unknown_truncates() {
        assert_eq!(compact_label("Something"), "Some");
        assert_eq!(compact_label("ab"), "ab");
        assert_eq!(compact_label("Ünïcødé"), "Ünïc");
    }

    #[test]
    fn draw_compact_height_fits_taskbar() {
        let now = Utc::now();
        let resets = now + chrono::Duration::hours(2);
        let claude = ProviderSnapshot {
            name: "Claude",
            plan: None,
            meters: vec![
                Meter {
                    label: "5h".into(),
                    percent: 40.0,
                    detail: None,
                    resets_at: Some(resets),
                },
                Meter {
                    label: "7d".into(),
                    percent: 55.0,
                    detail: None,
                    resets_at: None,
                },
                Meter {
                    label: "7d Opus".into(),
                    percent: 70.0,
                    detail: None,
                    resets_at: None,
                },
            ],
            status: ProviderStatus::Ok,
            fetched_at: now,
        };
        let cursor = ProviderSnapshot {
            name: "Cursor",
            plan: None,
            meters: vec![Meter {
                label: "Included".into(),
                percent: 30.0,
                detail: None,
                resets_at: None,
            }],
            status: ProviderStatus::Ok,
            fetched_at: now,
        };

        let ctx = egui::Context::default();
        let mut rect = Rect::NOTHING;
        let _ = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_width(500.0);
                let (r, _) = draw_compact(ui, Some(&claude), Some(&cursor), now, true);
                rect = r;
            });
        });

        assert!(rect.height() <= 2.0 * ROW_HEIGHT + 1.0);
        assert!(rect.width() > 100.0);
        assert!(rect.width() < 400.0);
    }

    #[test]
    fn draw_compact_loading_is_narrow() {
        let now = Utc::now();
        let ctx = egui::Context::default();
        let mut rect = Rect::NOTHING;
        let _ = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let (r, _) = draw_compact(ui, None, None, now, true);
                rect = r;
            });
        });

        assert!(rect.width() < 166.0);
    }

    #[test]
    fn arc_points_empty_at_zero() {
        let c = egui::pos2(10.0, 10.0);
        assert!(arc_points(c, 8.5, 0.0, 48).is_empty());
        assert!(arc_points(c, 8.5, -1.0, 48).is_empty());
    }

    #[test]
    fn arc_points_quarter_ends_at_three_oclock() {
        let center = egui::pos2(50.0, 50.0);
        let r = 8.5_f32;
        let pts = arc_points(center, r, 0.25, 48);
        let last = *pts.last().unwrap();
        assert!((last.x - (center.x + r)).abs() < 0.05);
        assert!((last.y - center.y).abs() < 0.05);
    }

    #[test]
    fn arc_points_full_closes() {
        let center = egui::pos2(0.0, 0.0);
        let pts = arc_points(center, 10.0, 1.0, 64);
        let first = pts.first().unwrap();
        let last = pts.last().unwrap();
        assert!((first.x - last.x).abs() < 0.05);
        assert!((first.y - last.y).abs() < 0.05);
    }

    #[test]
    fn draw_rings_height_fits() {
        let now = Utc::now();
        let claude = ProviderSnapshot {
            name: "Claude",
            plan: None,
            meters: vec![
                Meter {
                    label: "5h".into(),
                    percent: 40.0,
                    detail: None,
                    resets_at: None,
                },
                Meter {
                    label: "7d".into(),
                    percent: 55.0,
                    detail: None,
                    resets_at: None,
                },
            ],
            status: ProviderStatus::Ok,
            fetched_at: now,
        };
        let cursor = ProviderSnapshot {
            name: "Cursor",
            plan: None,
            meters: vec![
                Meter {
                    label: "Included".into(),
                    percent: 30.0,
                    detail: None,
                    resets_at: None,
                },
                Meter {
                    label: "Plan".into(),
                    percent: 60.0,
                    detail: None,
                    resets_at: None,
                },
            ],
            status: ProviderStatus::Ok,
            fetched_at: now,
        };

        let ctx = egui::Context::default();
        let mut rect = Rect::NOTHING;
        let _ = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_width(500.0);
                let (r, _) = draw_rings(ui, Some(&claude), Some(&cursor), now, true);
                rect = r;
            });
        });

        assert!(rect.height() <= 24.0);
    }

    #[test]
    fn draw_rings_loading_is_narrow() {
        let now = Utc::now();
        let ctx = egui::Context::default();
        let mut rect = Rect::NOTHING;
        let _ = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let (r, _) = draw_rings(ui, None, None, now, true);
                rect = r;
            });
        });

        assert!(rect.width() < 120.0);
    }
}
