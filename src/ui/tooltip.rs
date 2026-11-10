use eframe::egui::{self, Color32, FontId, Pos2, Rect, Vec2};

/// `IdTypeMap::remove_temp` requires `Default`. `egui::Rect` has no `Default`,
/// so the slot type is `Option<(String, Rect)>` and the stored value is `Some`.
type TooltipSlot = Option<(String, Rect)>;

fn tooltip_id() -> egui::Id {
    egui::Id::new("floating_tooltip")
}

/// Records the tooltip text when `response` is hovered this frame.
pub fn set(response: &egui::Response, text: impl Into<String>) {
    if !response.hovered() {
        return;
    }
    let text = text.into();
    let rect = response.rect;
    response.ctx.data_mut(|d| {
        d.insert_temp(tooltip_id(), Some((text, rect)) as TooltipSlot);
    });
}

/// Call once per frame at the end of WidgetApp::update (root viewport). If a tooltip was recorded,
/// shows it in a borderless viewport outside the widget window; then clears the slot.
pub fn show(ctx: &egui::Context) {
    let slot: Option<TooltipSlot> = ctx.data_mut(|d| d.remove_temp(tooltip_id()));
    let Some((text, widget_rect)) = slot.flatten() else {
        return;
    };

    let Some(inner) = ctx.input(|i| i.viewport().inner_rect) else {
        return;
    };
    let anchor = widget_rect.translate(inner.min.to_vec2());

    let galley = ctx.fonts(|f| {
        f.layout(
            text.clone(),
            FontId::proportional(12.0),
            Color32::from_gray(230),
            320.0,
        )
    });
    let padded = galley.size() + egui::vec2(16.0, 16.0);
    let size = egui::vec2(padded.x.ceil(), padded.y.ceil());

    let (sx, sy) = super::primary_screen_size();
    let pos = place(anchor, size, egui::vec2(sx, sy));

    let label = text;
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("ai-usage-tooltip"),
        egui::ViewportBuilder::default()
            .with_title("AI Usage tooltip")
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_active(false)
            .with_mouse_passthrough(true)
            .with_position(pos)
            .with_inner_size(size),
        |ctx, _| {
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(Color32::from_rgb(27, 27, 32))
                        .stroke(egui::Stroke::new(1.0, Color32::from_gray(70)))
                        .corner_radius(4.0)
                        .inner_margin(8.0),
                )
                .show(ctx, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    ui.set_max_width(320.0);
                    ui.label(
                        egui::RichText::new(label.clone())
                            .font(FontId::proportional(12.0))
                            .color(Color32::from_gray(230)),
                    );
                });
        },
    );
}

/// Pure placement: centered on `anchor` horizontally, 6 pt above it, or 6 pt below if it would go
/// above y=0; x clamped to [4, screen.x - size.x - 4]. All values in screen points.
pub(crate) fn place(anchor: Rect, size: Vec2, screen: Vec2) -> Pos2 {
    let mut x = anchor.center().x - size.x * 0.5;
    let above = anchor.min.y - size.y - 6.0;
    let y = if above < 0.0 {
        anchor.max.y + 6.0
    } else {
        above
    };
    let max_x = screen.x - size.x - 4.0;
    if max_x >= 4.0 {
        x = x.clamp(4.0, max_x);
    } else {
        x = 4.0;
    }
    egui::pos2(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_above_when_room() {
        let anchor = Rect::from_min_size(egui::pos2(100.0, 80.0), egui::vec2(40.0, 20.0));
        let size = egui::vec2(60.0, 24.0);
        let pos = place(anchor, size, egui::vec2(800.0, 600.0));
        assert_eq!(pos, egui::pos2(90.0, 50.0));
    }

    #[test]
    fn place_below_near_top_edge() {
        let anchor = Rect::from_min_size(egui::pos2(100.0, 10.0), egui::vec2(40.0, 18.0));
        let size = egui::vec2(60.0, 20.0);
        let pos = place(anchor, size, egui::vec2(800.0, 600.0));
        assert_eq!(pos, egui::pos2(90.0, 34.0));
    }

    #[test]
    fn place_clamped_at_left_edge() {
        let anchor = Rect::from_min_size(egui::pos2(0.0, 100.0), egui::vec2(10.0, 10.0));
        let size = egui::vec2(40.0, 16.0);
        let pos = place(anchor, size, egui::vec2(800.0, 600.0));
        assert_eq!(pos, egui::pos2(4.0, 78.0));
    }

    #[test]
    fn place_clamped_at_right_edge() {
        let anchor = Rect::from_min_size(egui::pos2(760.0, 100.0), egui::vec2(30.0, 10.0));
        let size = egui::vec2(50.0, 16.0);
        let pos = place(anchor, size, egui::vec2(800.0, 600.0));
        assert_eq!(pos, egui::pos2(746.0, 78.0));
    }
}
