use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Utc};
use eframe::egui::{self, Align2, Color32, Rect, Sense, Vec2};
use egui::FontId;

use crate::claude;
use crate::config::{self, Config, Layout};
use crate::cursor;
use crate::taskbar;
use crate::model::{
    format_countdown, Meter, ProviderSnapshot, ProviderStatus, Severity,
};

mod compact;
mod tray;

struct Shared {
    claude: Option<ProviderSnapshot>,
    cursor: Option<ProviderSnapshot>,
    refreshing: bool,
}

fn is_horizontal(cfg: &Config) -> bool {
    cfg.taskbar_mode || cfg.layout != Layout::Vertical
}

#[cfg(windows)]
fn system_dpi_scale() -> f32 {
    #[link(name = "user32")]
    extern "system" {
        fn GetDpiForSystem() -> u32;
    }
    unsafe {
        let dpi = GetDpiForSystem();
        if dpi == 0 {
            1.0
        } else {
            (dpi as f32 / 96.0).max(1.0)
        }
    }
}

#[cfg(not(windows))]
fn system_dpi_scale() -> f32 {
    1.0
}

fn initial_taskbar_pos(cfg: &Config, w: f32, h: f32) -> Option<egui::Pos2> {
    let info = taskbar::taskbar_info()?;
    let s = system_dpi_scale();
    let size_px = ((w * s).round() as i32, (h * s).round() as i32);
    let preferred = cfg.window_x.map(|x| (x * s).round() as i32);
    let (x, y) = taskbar::dock_position(&info, size_px, preferred)?;
    Some(egui::pos2(x as f32 / s, y as f32 / s))
}

pub fn run(cfg: Config) -> eframe::Result<()> {
    let horizontal = is_horizontal(&cfg);
    let (inner_w, inner_h) = if horizontal {
        (320.0_f32, 40.0_f32)
    } else {
        (280.0_f32, 150.0_f32)
    };
    let (min_w, min_h) = if horizontal {
        (80.0_f32, 30.0_f32)
    } else {
        (200.0_f32, 60.0_f32)
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("AI Usage")
        .with_inner_size([inner_w, inner_h])
        .with_min_inner_size([min_w, min_h])
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_taskbar(false);

    if cfg.always_on_top || cfg.taskbar_mode {
        viewport = viewport.with_always_on_top();
    }

    if cfg.taskbar_mode {
        if let Some(pos) = initial_taskbar_pos(&cfg, inner_w, inner_h) {
            viewport = viewport.with_position(pos);
        } else if let (Some(x), Some(y)) = (cfg.window_x, cfg.window_y) {
            viewport = viewport.with_position(egui::pos2(x, y));
        } else {
            viewport = viewport.with_position(default_top_right(inner_w, inner_h));
        }
    } else if let (Some(x), Some(y)) = (cfg.window_x, cfg.window_y) {
        viewport = viewport.with_position(egui::pos2(x, y));
    } else {
        viewport = viewport.with_position(default_top_right(inner_w, inner_h));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let cfg = Arc::new(Mutex::new(cfg));
    let cfg_for_app = Arc::clone(&cfg);

    eframe::run_native(
        "ai-usage-widget",
        options,
        Box::new(move |cc| {
            Ok(Box::new(WidgetApp::new(cc, cfg_for_app)))
        }),
    )
}

fn default_top_right(width: f32, _height: f32) -> egui::Pos2 {
    let margin = 12.0;
    let (screen_w, _) = primary_screen_size();
    egui::pos2((screen_w - width - margin).max(margin), margin)
}

#[cfg(windows)]
fn primary_screen_size() -> (f32, f32) {
    #[link(name = "user32")]
    extern "system" {
        fn GetSystemMetrics(n_index: i32) -> i32;
    }
    const SM_CXSCREEN: i32 = 0;
    const SM_CYSCREEN: i32 = 1;
    unsafe {
        let cx = GetSystemMetrics(SM_CXSCREEN) as f32;
        let cy = GetSystemMetrics(SM_CYSCREEN) as f32;
        let scale = system_dpi_scale();
        (cx / scale, cy / scale)
    }
}

#[cfg(not(windows))]
fn primary_screen_size() -> (f32, f32) {
    (1920.0, 1080.0)
}

fn spawn_worker(
    shared: Arc<Mutex<Shared>>,
    cfg: Arc<Mutex<Config>>,
    ctx: egui::Context,
    rx: mpsc::Receiver<()>,
    poll: Duration,
) {
    thread::spawn(move || {
        let mut claude_poller = claude::Poller::new();
        loop {
            {
                let mut s = shared.lock().expect("shared lock");
                s.refreshing = true;
            }

            let cfg_disk = config::load();
            if let Ok(mut c) = cfg.lock() {
                c.transparent_background = cfg_disk.transparent_background;
            }
            let claude_snap = claude_poller.poll();
            let cursor_snap = cursor::fetch(&cfg_disk);

            {
                let mut s = shared.lock().expect("shared lock");
                s.claude = Some(claude_snap);
                s.cursor = Some(cursor_snap);
                s.refreshing = false;
            }
            ctx.request_repaint();

            match rx.recv_timeout(poll) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

fn spawn_taskbar_keeper(flag: Arc<AtomicBool>) {
    thread::spawn(move || {
        let mut hwnd: Option<isize> = None;
        let mut hidden = false;
        let mut owned = false;
        loop {
            thread::sleep(Duration::from_millis(250));
            if hwnd.is_none() {
                hwnd = taskbar::find_own_window();
                if hwnd.is_none() {
                    continue;
                }
            }
            let h = hwnd.unwrap();
            if !flag.load(Ordering::Relaxed) {
                if hidden {
                    taskbar::set_visible(h, true);
                    hidden = false;
                }
                if owned {
                    taskbar::set_owned_by_taskbar(h, false);
                    owned = false;
                }
                continue;
            }
            if !owned {
                taskbar::set_owned_by_taskbar(h, true);
                owned = true;
            }
            if taskbar::fullscreen_app_active() {
                if !hidden {
                    taskbar::set_visible(h, false);
                    hidden = true;
                }
            } else {
                if hidden {
                    taskbar::set_visible(h, true);
                    hidden = false;
                }
                taskbar::keep_on_top(h);
            }
        }
    });
}

struct WidgetApp {
    shared: Arc<Mutex<Shared>>,
    refresh_tx: mpsc::Sender<()>,
    cfg: Arc<Mutex<Config>>,
    last_saved_pos: Option<(f32, f32)>,
    last_save_at: Option<Instant>,
    quit_pending: bool,
    taskbar_flag: Arc<AtomicBool>,
    tray: Option<tray::Tray>,
}

impl WidgetApp {
    fn new(cc: &eframe::CreationContext<'_>, cfg: Arc<Mutex<Config>>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let poll_secs = cfg.lock().expect("cfg lock").poll_seconds.max(30);
        let poll = Duration::from_secs(poll_secs);

        let shared = Arc::new(Mutex::new(Shared {
            claude: None,
            cursor: None,
            refreshing: true,
        }));

        let (refresh_tx, refresh_rx) = mpsc::channel();
        spawn_worker(
            Arc::clone(&shared),
            Arc::clone(&cfg),
            cc.egui_ctx.clone(),
            refresh_rx,
            poll,
        );

        let taskbar_mode = cfg.lock().expect("cfg lock").taskbar_mode;
        let taskbar_flag = Arc::new(AtomicBool::new(taskbar_mode));
        spawn_taskbar_keeper(Arc::clone(&taskbar_flag));

        let tray = match tray::Tray::new(&cc.egui_ctx) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("tray icon: {e}");
                None
            }
        };

        Self {
            shared,
            refresh_tx,
            cfg,
            last_saved_pos: None,
            last_save_at: None,
            quit_pending: false,
            taskbar_flag,
            tray,
        }
    }

    fn save_config_with(&self, mut f: impl FnMut(&mut Config)) {
        let mut disk = config::load();
        f(&mut disk);
        let _ = config::save(&disk);
        if let Ok(mut c) = self.cfg.lock() {
            f(&mut c);
        }
    }

    fn action_refresh(&self) {
        let _ = self.refresh_tx.send(());
    }

    fn action_set_always_on_top(
        &mut self,
        ctx: &egui::Context,
        on: bool,
        root_outer: Option<Rect>,
    ) {
        self.save_config_with(|c| c.always_on_top = on);
        let level = if on {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        };
        ctx.send_viewport_cmd_to(
            egui::ViewportId::ROOT,
            egui::ViewportCommand::WindowLevel(level),
        );
        if let Some(outer) = root_outer {
            self.save_window_fields(outer.min.x, outer.min.y, on);
        }
    }

    fn action_set_layout(&mut self, ctx: &egui::Context, layout: Layout) {
        if self
            .cfg
            .lock()
            .map(|c| c.taskbar_mode && layout == Layout::Vertical)
            .unwrap_or(false)
        {
            return;
        }
        let was_horizontal = self
            .cfg
            .lock()
            .map(|c| is_horizontal(&c))
            .unwrap_or(false);
        self.save_config_with(|c| c.layout = layout);
        let now_horizontal = self
            .cfg
            .lock()
            .map(|c| is_horizontal(&c))
            .unwrap_or(false);
        if was_horizontal != now_horizontal {
            if now_horizontal {
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::MinInnerSize(Vec2::new(80.0, 30.0)),
                );
            } else {
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::MinInnerSize(Vec2::new(200.0, 60.0)),
                );
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::InnerSize(Vec2::new(280.0, 150.0)),
                );
            }
        }
    }

    fn action_set_taskbar_mode(&mut self, ctx: &egui::Context, on: bool) {
        let was_horizontal = self
            .cfg
            .lock()
            .map(|c| is_horizontal(&c))
            .unwrap_or(false);
        self.save_config_with(|c| {
            c.taskbar_mode = on;
            if on {
                c.always_on_top = true;
            }
        });
        let now_horizontal = self
            .cfg
            .lock()
            .map(|c| is_horizontal(&c))
            .unwrap_or(false);
        if was_horizontal != now_horizontal {
            if now_horizontal {
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::MinInnerSize(Vec2::new(80.0, 30.0)),
                );
            } else {
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::MinInnerSize(Vec2::new(200.0, 60.0)),
                );
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::InnerSize(Vec2::new(280.0, 150.0)),
                );
            }
        }
        self.taskbar_flag.store(on, Ordering::Relaxed);
        if on {
            ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop),
            );
        }
    }

    fn action_open_config(&self) {
        if let Some(path) = config::config_path() {
            let _ = std::process::Command::new("notepad.exe")
                .arg(path)
                .spawn();
        }
    }

    fn action_quit(&mut self, root_outer: Option<Rect>) {
        if let Some(outer) = root_outer {
            let on_top = self.cfg.lock().map(|c| c.always_on_top).unwrap_or(true);
            self.save_window_fields(outer.min.x, outer.min.y, on_top);
        }
        self.quit_pending = true;
    }

    fn action_set_transparent_background(&mut self, ctx: &egui::Context, on: bool) {
        self.save_config_with(|c| c.transparent_background = on);
        ctx.request_repaint();
    }

    fn snap_to_taskbar(&self, ctx: &egui::Context) {
        if ctx.input(|i| i.pointer.primary_down()) {
            return;
        }
        let Some(outer) = ctx.input(|i| i.viewport().outer_rect) else {
            return;
        };
        let ppp = ctx
            .input(|i| i.viewport().native_pixels_per_point)
            .unwrap_or(1.0);
        let Some(info) = taskbar::taskbar_info() else {
            return;
        };
        let size_px = (
            (outer.width() * ppp).round() as i32,
            (outer.height() * ppp).round() as i32,
        );
        let x_px = (outer.min.x * ppp).round() as i32;
        let Some((tx, ty)) = taskbar::dock_position(&info, size_px, Some(x_px)) else {
            return;
        };
        let target = egui::pos2(tx as f32 / ppp, ty as f32 / ppp);
        if (target - outer.min).length() > 1.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(target));
        }
    }

    fn save_window_fields(&self, x: f32, y: f32, always_on_top: bool) {
        let mut disk = config::load();
        disk.window_x = Some(x);
        disk.window_y = Some(y);
        disk.always_on_top = always_on_top;
        if config::save(&disk).is_err() {
            return;
        }

        if let Ok(mut c) = self.cfg.lock() {
            c.window_x = Some(x);
            c.window_y = Some(y);
            c.always_on_top = always_on_top;
        }
    }

    fn persist_position_if_needed(&mut self, ctx: &egui::Context) {
        let outer = ctx.input(|i| i.viewport().outer_rect);
        let Some(outer) = outer else {
            return;
        };

        let pointer_down = ctx.input(|i| i.pointer.primary_down());
        if pointer_down {
            return;
        }

        let x = outer.min.x;
        let y = outer.min.y;
        let always_on_top = self.cfg.lock().map(|c| c.always_on_top).unwrap_or(true);

        let moved = match self.last_saved_pos {
            Some((lx, ly)) => (x - lx).abs() > 1.0 || (y - ly).abs() > 1.0,
            None => true,
        };

        if !moved {
            return;
        }

        let now = Instant::now();
        if let Some(last) = self.last_save_at {
            if now.duration_since(last) < Duration::from_secs(2) {
                return;
            }
        }

        self.save_window_fields(x, y, always_on_top);
        self.last_saved_pos = Some((x, y));
        self.last_save_at = Some(now);
    }
}

impl eframe::App for WidgetApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.quit_pending {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let tray_actions = self.tray.as_ref().map(|t| t.poll_actions());
        let mut quit_from_tray = false;
        if let Some(actions) = tray_actions {
            let root_outer = ctx.input(|i| i.viewport().outer_rect);
            for action in actions {
                let (always_on_top, taskbar_mode, transparent_background) = self
                    .cfg
                    .lock()
                    .map(|c| (c.always_on_top, c.taskbar_mode, c.transparent_background))
                    .unwrap_or((true, false, false));
                match action {
                    tray::TrayAction::Refresh => self.action_refresh(),
                    tray::TrayAction::ToggleAlwaysOnTop if !taskbar_mode => {
                        self.action_set_always_on_top(ctx, !always_on_top, root_outer);
                    }
                    tray::TrayAction::SetLayout(layout) => self.action_set_layout(ctx, layout),
                    tray::TrayAction::ToggleTaskbarMode => {
                        self.action_set_taskbar_mode(ctx, !taskbar_mode);
                    }
                    tray::TrayAction::ToggleTransparent => {
                        self.action_set_transparent_background(ctx, !transparent_background);
                    }
                    tray::TrayAction::ToggleAlwaysOnTop => {}
                    tray::TrayAction::OpenConfig => self.action_open_config(),
                    tray::TrayAction::Quit => {
                        self.action_quit(root_outer);
                        quit_from_tray = true;
                    }
                }
            }
        }
        if quit_from_tray {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if let Some(tray) = self.tray.as_ref() {
            let (always_on_top, layout, taskbar_mode, transparent_background) = self
                .cfg
                .lock()
                .map(|c| {
                    let layout = c.layout;
                    let effective = if c.taskbar_mode && layout == Layout::Vertical {
                        Layout::Horizontal
                    } else {
                        layout
                    };
                    (
                        c.always_on_top,
                        effective,
                        c.taskbar_mode,
                        c.transparent_background,
                    )
                })
                .unwrap_or((true, Layout::Vertical, false, false));
            tray.sync(
                always_on_top,
                layout,
                taskbar_mode,
                transparent_background,
            );
        }

        let (horizontal, taskbar_mode, transparent_background, layout) = self
            .cfg
            .lock()
            .map(|c| (is_horizontal(&c), c.taskbar_mode, c.transparent_background, c.layout))
            .unwrap_or((false, false, false, Layout::Vertical));

        let (fill, stroke) = if transparent_background {
            (Color32::TRANSPARENT, egui::Stroke::NONE)
        } else {
            (
                Color32::from_rgba_unmultiplied(18, 18, 22, 245),
                egui::Stroke::new(1.0, Color32::from_gray(55)),
            )
        };

        let panel_frame = if horizontal {
            egui::Frame::new()
                .fill(fill)
                .stroke(stroke)
                .corner_radius(3.0)
                .inner_margin(4.0)
        } else {
            egui::Frame::new()
                .fill(fill)
                .stroke(stroke)
                .corner_radius(4.0)
                .inner_margin(8.0)
        };

        let mut content_bottom = 0.0_f32;
        let mut compact_rect = Rect::NOTHING;

        egui::CentralPanel::default().frame(panel_frame).show(ctx, |ui| {
            let bg = ui.interact(
                ui.max_rect(),
                egui::Id::new("bg"),
                Sense::click_and_drag(),
            );
            if bg.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }

            let (claude, cursor, refreshing) = {
                let s = self.shared.lock().expect("shared lock");
                (s.claude.clone(), s.cursor.clone(), s.refreshing)
            };

            let now = Utc::now();

            if horizontal {
                let (r, refresh_clicked) = if layout == Layout::Rings {
                    compact::draw_rings(ui, claude.as_ref(), cursor.as_ref(), now, refreshing)
                } else {
                    compact::draw_compact(ui, claude.as_ref(), cursor.as_ref(), now, refreshing)
                };
                compact_rect = r;
                if refresh_clicked {
                    self.action_refresh();
                }
            } else {
                let header_font = FontId::proportional(13.0);
                let body_font = FontId::proportional(12.0);
                ui.style_mut().override_font_id = Some(body_font.clone());

                let content_rect = ui
                    .vertical(|ui| {
                        draw_provider(ui, &header_font, &body_font, claude.as_ref(), now);
                        draw_provider(ui, &header_font, &body_font, cursor.as_ref(), now);

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(footer_updated(
                                    claude.as_ref(),
                                    cursor.as_ref(),
                                ))
                                .color(Color32::GRAY)
                                .small(),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let sense = if refreshing {
                                        Sense::hover()
                                    } else {
                                        Sense::click()
                                    };
                                    let (rect, response) =
                                        ui.allocate_exact_size(Vec2::new(14.0, 14.0), sense);
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
                                        rect.center(),
                                        Align2::CENTER_CENTER,
                                        "⟳",
                                        FontId::proportional(11.0),
                                        color,
                                    );
                                    if refresh_clicked {
                                        self.action_refresh();
                                    }
                                },
                            );
                        });
                    })
                    .response
                    .rect;
                content_bottom = content_rect.max.y;
            }
        });

        if horizontal {
            let desired = Vec2::new(
                (compact_rect.max.x + 4.0).max(80.0).ceil(),
                (compact_rect.max.y + 4.0).max(30.0).ceil(),
            );
            let cur = ctx.input(|i| i.viewport().inner_rect);
            if let Some(r) = cur {
                if (r.width() - desired.x).abs() > 2.0 || (r.height() - desired.y).abs() > 2.0 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(desired));
                }
            }
        } else {
            let desired_h = (content_bottom + 8.0).max(60.0);
            let cur_h = ctx.input(|i| i.viewport().inner_rect.map(|r| r.height()));
            if let Some(h) = cur_h {
                if (h - desired_h).abs() > 2.0 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                        280.0, desired_h,
                    )));
                }
            }
        }

        if taskbar_mode {
            self.snap_to_taskbar(ctx);
        }
        self.persist_position_if_needed(ctx);
        if taskbar_mode {
            ctx.request_repaint_after(Duration::from_secs(1));
        } else {
            ctx.request_repaint_after(Duration::from_secs(30));
        }
    }
}

fn footer_updated(
    claude: Option<&ProviderSnapshot>,
    cursor: Option<&ProviderSnapshot>,
) -> String {
    let newest: Option<DateTime<Utc>> = claude
        .map(|s| s.fetched_at)
        .into_iter()
        .chain(cursor.map(|s| s.fetched_at))
        .max();
    match newest {
        Some(t) => {
            let local: DateTime<Local> = t.into();
            format!("updated {}", local.format("%H:%M"))
        }
        None => "updated —".to_string(),
    }
}

fn draw_provider(
    ui: &mut egui::Ui,
    header_font: &FontId,
    body_font: &FontId,
    snapshot: Option<&ProviderSnapshot>,
    now: DateTime<Utc>,
) {
    let Some(s) = snapshot else {
        ui.label(egui::RichText::new("loading…").color(Color32::GRAY).font(body_font.clone()));
        ui.add_space(6.0);
        return;
    };

    ui.horizontal(|ui| {
        let mut title = s.name.to_string();
        if let Some(ref plan) = s.plan {
            title.push_str(&format!(" · {plan}"));
        }
        ui.label(
            egui::RichText::new(title)
                .strong()
                .font(header_font.clone()),
        );

        if s.status != ProviderStatus::Ok {
            let (text, color) = match &s.status {
                ProviderStatus::NotSignedIn(msg) => (msg.as_str(), Color32::GRAY),
                ProviderStatus::Error(msg) => (msg.as_str(), Color32::from_rgb(229, 57, 53)),
                ProviderStatus::Stale(msg) => (msg.as_str(), Color32::from_rgb(255, 179, 0)),
                ProviderStatus::Ok => ("", Color32::GRAY),
            };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(text)
                        .font(body_font.clone())
                        .color(color),
                );
            });
        }
    });

    if s.status.has_meters() {
        for m in &s.meters {
            draw_meter_row(ui, body_font, m, now);
        }
    }

    ui.add_space(6.0);
}

fn draw_meter_row(ui: &mut egui::Ui, body_font: &FontId, m: &Meter, now: DateTime<Utc>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;

        let (r, _) = ui.allocate_exact_size(egui::vec2(58.0, 14.0), Sense::hover());
        ui.painter().text(
            r.left_center(),
            Align2::LEFT_CENTER,
            &m.label,
            body_font.clone(),
            ui.visuals().text_color(),
        );

        let bar_size = egui::vec2(100.0, 6.0);
        let (rect, response) = ui.allocate_exact_size(bar_size, Sense::hover());
        paint_meter_bar(ui.painter(), rect, m.percent);
        if let Some(ref detail) = m.detail {
            response.on_hover_text(detail);
        }

        let (r, _) = ui.allocate_exact_size(egui::vec2(34.0, 14.0), Sense::hover());
        let pct = format!("{:.0}%", m.percent);
        ui.painter().text(
            r.right_center(),
            Align2::RIGHT_CENTER,
            pct,
            body_font.clone(),
            ui.visuals().text_color(),
        );

        if let Some(resets) = m.resets_at {
            let cd = format_countdown(resets, now);
            ui.label(
                egui::RichText::new(cd)
                    .font(body_font.clone())
                    .color(Color32::GRAY),
            );
        }
    });
}

fn paint_meter_bar(painter: &egui::Painter, rect: Rect, percent: f32) {
    let track = Color32::from_gray(60);
    let fill = severity_color(Severity::from_percent(percent));
    let corner = 3.0;
    painter.rect_filled(rect, corner, track);
    let fill_w = rect.width() * (percent.clamp(0.0, 100.0) / 100.0);
    if fill_w > 0.0 {
        let fill_rect = Rect::from_min_size(rect.min, egui::vec2(fill_w, rect.height()));
        painter.rect_filled(fill_rect, corner, fill);
    }
}

pub fn severity_color(s: Severity) -> Color32 {
    match s {
        Severity::Normal => Color32::from_rgb(76, 175, 80),
        Severity::Warning => Color32::from_rgb(255, 179, 0),
        Severity::Critical => Color32::from_rgb(229, 57, 53),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Layout;

    #[test]
    fn severity_colors() {
        assert_eq!(severity_color(Severity::Normal), Color32::from_rgb(76, 175, 80));
        assert_eq!(severity_color(Severity::Warning), Color32::from_rgb(255, 179, 0));
        assert_eq!(severity_color(Severity::Critical), Color32::from_rgb(229, 57, 53));
    }

    #[test]
    fn is_horizontal_combinations() {
        let v = Config {
            layout: Layout::Vertical,
            taskbar_mode: false,
            ..Default::default()
        };
        assert!(!is_horizontal(&v));

        let h = Config {
            layout: Layout::Horizontal,
            taskbar_mode: false,
            ..Default::default()
        };
        assert!(is_horizontal(&h));

        let tb = Config {
            layout: Layout::Vertical,
            taskbar_mode: true,
            ..Default::default()
        };
        assert!(is_horizontal(&tb));

        let both = Config {
            layout: Layout::Horizontal,
            taskbar_mode: true,
            ..Default::default()
        };
        assert!(is_horizontal(&both));

        let rings = Config {
            layout: Layout::Rings,
            taskbar_mode: false,
            ..Default::default()
        };
        assert!(is_horizontal(&rings));

        let rings_tb = Config {
            layout: Layout::Rings,
            taskbar_mode: true,
            ..Default::default()
        };
        assert!(is_horizontal(&rings_tb));
    }
}
