use chrono::{DateTime, Local, TimeZone, Utc};
use eframe::egui::{self, Align2, Color32, Rect, Sense};
use egui::FontId;

use crate::model::{format_countdown, Meter, ProviderSnapshot, ProviderStatus, Severity};

pub const ROW_HEIGHT: f32 = 16.0;
pub const MAX_METERS: usize = 3;

fn icons_percent_font() -> FontId {
    FontId::proportional(10.0)
}

fn icons_column_width(ui: &egui::Ui) -> f32 {
    let font = icons_percent_font();
    let color = ui.visuals().text_color();
    ui.fonts(|f| f.layout_no_wrap("100%".into(), font, color).size().x)
        .ceil()
        + 2.0
}

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
        super::tooltip::set(&response, tip);
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
    super::tooltip::set(&response, hover);

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

/// Draws the two rows top-to-bottom and returns the union rect.
pub fn draw_compact(
    ui: &mut egui::Ui,
    claude: Option<&ProviderSnapshot>,
    cursor: Option<&ProviderSnapshot>,
    now: DateTime<Utc>,
) -> egui::Rect {
    let mut union = Rect::NOTHING;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        union = union.union(draw_compact_row(ui, "Claude", claude, now));
        union = union.union(draw_compact_row(ui, "Cursor", cursor, now));
    });
    union
}

fn paint_svg_icon(
    ui: &mut egui::Ui,
    source: egui::ImageSource<'static>,
    size: egui::Vec2,
    height: f32,
) -> egui::Response {
    let (slot, response) = ui.allocate_exact_size(egui::vec2(size.x, height), Sense::hover());
    let image_rect = Rect::from_center_size(slot.center(), size);
    let mut icon_ui = ui.new_child(egui::UiBuilder::new().max_rect(image_rect));
    let _ = icon_ui.add(
        egui::Image::new(source)
            .fit_to_exact_size(size)
            .sense(Sense::hover()),
    );
    response
}

fn paint_icons_status_cell(
    ui: &mut egui::Ui,
    text: &str,
    color: Color32,
    hover: Option<&str>,
    height: f32,
) -> Rect {
    let font = body_font();
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), color);
    let size = egui::vec2(galley.size().x, height);
    let (r, response) = ui.allocate_exact_size(size, Sense::hover());
    let pos = egui::pos2(r.left(), r.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, color);
    if let Some(tip) = hover {
        super::tooltip::set(&response, tip);
    }
    r
}

fn paint_icons_column(
    ui: &mut egui::Ui,
    provider: &str,
    m: &Meter,
    now: DateTime<Utc>,
    height: f32,
    col_w: f32,
) -> Rect {
    let font = icons_percent_font();
    let text_color = ui.visuals().text_color();
    let pct_text = format!("{:.0}%", m.percent);
    let text_galley = ui
        .painter()
        .layout_no_wrap(pct_text, font, text_color);
    let text_h = text_galley.size().y;
    let gap = 2.0;

    let (rect, response) = ui.allocate_exact_size(egui::vec2(col_w, height), Sense::hover());
    let painter = ui.painter();

    let text_x = rect.center().x - text_galley.size().x / 2.0;
    painter.galley(egui::pos2(text_x, rect.min.y), text_galley, text_color);

    let bar_top = rect.min.y + text_h + gap;
    let bar_rect = Rect::from_min_max(
        egui::pos2(rect.min.x, bar_top),
        egui::pos2(rect.max.x, rect.max.y),
    );
    painter.rect_filled(bar_rect, 0.0, Color32::from_gray(60));
    let fill_h = bar_rect.height() * (m.percent / 100.0).clamp(0.0, 1.0);
    if fill_h > 0.0 {
        let fill_rect = Rect::from_min_size(
            egui::pos2(bar_rect.min.x, bar_rect.max.y - fill_h),
            egui::vec2(bar_rect.width(), fill_h),
        );
        painter.rect_filled(
            fill_rect,
            0.0,
            super::severity_color(Severity::from_percent(m.percent)),
        );
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
    super::tooltip::set(&response, hover);
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

fn draw_icons_provider(
    ui: &mut egui::Ui,
    provider: &str,
    snapshot: Option<&ProviderSnapshot>,
    now: DateTime<Utc>,
    icon: egui::ImageSource<'static>,
    icon_size: egui::Vec2,
    height: f32,
    col_w: f32,
) -> Rect {
    let icon_resp = paint_svg_icon(ui, icon, icon_size, height);
    let mut union = icon_resp.rect;

    match snapshot {
        None => {
            union = union.union(paint_icons_status_cell(
                ui,
                "…",
                Color32::GRAY,
                Some("Loading"),
                height,
            ));
        }
        Some(s) => {
            super::tooltip::set(&icon_resp, provider_icon_hover(provider, s));
            match &s.status {
                ProviderStatus::NotSignedIn(msg) => {
                    union = union.union(paint_icons_status_cell(
                        ui,
                        "⚠",
                        Color32::GRAY,
                        Some(msg.as_str()),
                        height,
                    ));
                }
                ProviderStatus::Error(msg) => {
                    let error_red = Color32::from_rgb(229, 57, 53);
                    union = union.union(paint_icons_status_cell(
                        ui,
                        "⚠",
                        error_red,
                        Some(msg.as_str()),
                        height,
                    ));
                }
                ProviderStatus::Ok => {
                    ui.add_space(6.0);
                    for (i, m) in s.meters.iter().take(MAX_METERS).enumerate() {
                        if i > 0 {
                            ui.add_space(4.0);
                        }
                        union = union.union(paint_icons_column(ui, provider, m, now, height, col_w));
                    }
                }
                ProviderStatus::Stale(msg) => {
                    ui.add_space(6.0);
                    for (i, m) in s.meters.iter().take(MAX_METERS).enumerate() {
                        if i > 0 {
                            ui.add_space(4.0);
                        }
                        union = union.union(paint_icons_column(ui, provider, m, now, height, col_w));
                    }
                    let amber = Color32::from_rgb(255, 179, 0);
                    union = union.union(paint_icons_status_cell(
                        ui,
                        "⚠",
                        amber,
                        Some(msg.as_str()),
                        height,
                    ));
                }
            }
        }
    }

    union
}

/// One-row strip: Claude and Cursor logos with vertical usage bars.
/// Returns the union rect of everything drawn.
pub fn draw_icons(
    ui: &mut egui::Ui,
    claude: Option<&ProviderSnapshot>,
    cursor: Option<&ProviderSnapshot>,
    now: DateTime<Utc>,
    height: f32,
) -> egui::Rect {
    let mut union = Rect::NOTHING;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let col_w = icons_column_width(ui);

        union = union.union(draw_icons_provider(
            ui,
            "Claude",
            claude,
            now,
            egui::include_image!("../../assets/claude.svg"),
            egui::vec2(24.0, 15.0),
            height,
            col_w,
        ));

        ui.add_space(24.0);

        union = union.union(draw_icons_provider(
            ui,
            "Cursor",
            cursor,
            now,
            egui::include_image!("../../assets/cursor.svg"),
            egui::vec2(14.0, 16.0),
            height,
            col_w,
        ));
    });

    union
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
                let r = draw_compact(ui, Some(&claude), Some(&cursor), now);
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
                let r = draw_compact(ui, None, None, now);
                rect = r;
            });
        });

        assert!(rect.width() < 150.0);
    }

    fn icons_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        ctx
    }

    #[test]
    fn draw_icons_height_fits() {
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

        let ctx = icons_ctx();
        let mut rect = Rect::NOTHING;
        let _ = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_width(500.0);
                let r = draw_icons(ui, Some(&claude), Some(&cursor), now, 40.0);
                rect = r;
            });
        });

        assert!(rect.height() <= 40.5);
    }

    #[test]
    fn draw_icons_loading_is_narrow() {
        let now = Utc::now();
        let ctx = icons_ctx();
        let mut rect = Rect::NOTHING;
        let _ = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let r = draw_icons(ui, None, None, now, 40.0);
                rect = r;
            });
        });

        assert!(rect.width() < 134.0);
    }

    #[test]
    fn draw_icons_columns_fit_percent_text() {
        let now = Utc::now();
        let claude = ProviderSnapshot {
            name: "Claude",
            plan: None,
            meters: vec![Meter {
                label: "5h".into(),
                percent: 100.0,
                detail: None,
                resets_at: None,
            }],
            status: ProviderStatus::Ok,
            fetched_at: now,
        };

        let ctx = icons_ctx();
        let mut rect = Rect::NOTHING;
        let icon_w = 24.0_f32;
        let mut pct_w = 0.0_f32;
        let _ = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let font = icons_percent_font();
                let color = ui.visuals().text_color();
                pct_w = ui
                    .fonts(|f| f.layout_no_wrap("100%".into(), font, color).size().x)
                    .ceil();
                let r = draw_icons(ui, Some(&claude), None, now, 40.0);
                rect = r;
            });
        });

        assert!(rect.width() >= icon_w + 6.0 + pct_w);
    }
}
