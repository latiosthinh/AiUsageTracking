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

struct Shared {
    claude: Option<ProviderSnapshot>,
    cursor: Option<ProviderSnapshot>,
    refreshing: bool,
}

fn is_horizontal(cfg: &Config) -> bool {
    cfg.taskbar_mode || cfg.layout == Layout::Horizontal
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

            let cfg = config::load();
            let claude_snap = claude_poller.poll();
            let cursor_snap = cursor::fetch(&cfg);

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
        loop {
            thread::sleep(Duration::from_secs(1));
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
                continue;
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

/// Initial guess at the popup menu's size (points); corrected after the first frame.
const MENU_SIZE_ESTIMATE: Vec2 = Vec2::new(170.0, 150.0);
const MENU_GAP: f32 = 4.0;

/// Right-click menu shown in its own borderless window, used when the main
/// window is a thin horizontal strip that would clip an inline menu.
struct PopupMenu {
    /// Screen x (points) of the right-click.
    anchor_x: f32,
    /// Main window's outer rect (points) when the menu was opened.
    strip: Rect,
    /// Measured size of the menu window (points).
    size: Vec2,
    /// The popup has had keyboard focus at least once; losing it closes the menu.
    seen_focus: bool,
}

impl PopupMenu {
    /// Above the strip when there is room (taskbar at the bottom), else below it;
    /// clamped horizontally to the screen.
    fn position(&self) -> egui::Pos2 {
        let (screen_w, screen_h) = primary_screen_size();
        let x = (self.anchor_x - 8.0)
            .min(screen_w - self.size.x - MENU_GAP)
            .max(MENU_GAP);
        let above = self.strip.min.y - self.size.y - MENU_GAP;
        let below = self.strip.max.y + MENU_GAP;
        let y = if above >= 0.0 || below + self.size.y > screen_h {
            above.max(0.0)
        } else {
            below
        };
        egui::pos2(x, y)
    }
}

struct WidgetApp {
    menu: Option<PopupMenu>,
    shared: Arc<Mutex<Shared>>,
    refresh_tx: mpsc::Sender<()>,
    cfg: Arc<Mutex<Config>>,
    last_saved_pos: Option<(f32, f32)>,
    last_save_at: Option<Instant>,
    quit_pending: bool,
    taskbar_flag: Arc<AtomicBool>,
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
        spawn_worker(Arc::clone(&shared), cc.egui_ctx.clone(), refresh_rx, poll);

        let taskbar_mode = cfg.lock().expect("cfg lock").taskbar_mode;
        let taskbar_flag = Arc::new(AtomicBool::new(taskbar_mode));
        spawn_taskbar_keeper(Arc::clone(&taskbar_flag));

        Self {
            menu: None,
            shared,
            refresh_tx,
            cfg,
            last_saved_pos: None,
            last_save_at: None,
            quit_pending: false,
            taskbar_flag,
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

    /// Right-click menu items. Viewport commands always target the main window,
    /// so this works both in an inline context menu and in the popup window.
    /// Returns true when the menu should close.
    fn menu_items(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        taskbar_mode: bool,
        root_outer: Option<Rect>,
    ) -> bool {
        let mut close = false;
        if ui.button("Refresh now").clicked() {
            let _ = self.refresh_tx.send(());
            close = true;
        }

        let mut on_top = self.cfg.lock().map(|c| c.always_on_top).unwrap_or(true);
        if ui
            .add_enabled(
                !taskbar_mode,
                egui::Checkbox::new(&mut on_top, "Always on top"),
            )
            .changed()
        {
            self.save_config_with(|c| c.always_on_top = on_top);
            let level = if on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            };
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::WindowLevel(level));
            if let Some(outer) = root_outer {
                self.save_window_fields(outer.min.x, outer.min.y, on_top);
            }
            close = true;
        }

        let mut layout_horizontal = self
            .cfg
            .lock()
            .map(|c| c.layout == Layout::Horizontal)
            .unwrap_or(false);
        if ui
            .add_enabled(
                !taskbar_mode,
                egui::Checkbox::new(&mut layout_horizontal, "Horizontal layout"),
            )
            .changed()
        {
            let was_horizontal = self
                .cfg
                .lock()
                .map(|c| is_horizontal(&c))
                .unwrap_or(false);
            self.save_config_with(|c| {
                c.layout = if layout_horizontal {
                    Layout::Horizontal
                } else {
                    Layout::Vertical
                };
            });
            let now_horizontal = layout_horizontal;
            if was_horizontal != now_horizontal {
                if now_horizontal {
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::MinInnerSize(Vec2::new(
                        80.0, 30.0,
                    )));
                } else {
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::MinInnerSize(Vec2::new(
                        200.0, 60.0,
                    )));
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::InnerSize(Vec2::new(
                        280.0, 150.0,
                    )));
                }
            }
            close = true;
        }

        let mut tb_mode = taskbar_mode;
        if ui
            .checkbox(&mut tb_mode, "Taskbar mode")
            .changed()
        {
            let was_horizontal = self
                .cfg
                .lock()
                .map(|c| is_horizontal(&c))
                .unwrap_or(false);
            let on = tb_mode;
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
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::MinInnerSize(Vec2::new(
                        80.0, 30.0,
                    )));
                } else {
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::MinInnerSize(Vec2::new(
                        200.0, 60.0,
                    )));
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::InnerSize(Vec2::new(
                        280.0, 150.0,
                    )));
                }
            }
            self.taskbar_flag.store(on, Ordering::Relaxed);
            if on {
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::WindowLevel(
                    egui::WindowLevel::AlwaysOnTop,
                ));
            }
            close = true;
        }

        if ui.button("Open config file").clicked() {
            if let Some(path) = config::config_path() {
                let _ = std::process::Command::new("notepad.exe")
                    .arg(path)
                    .spawn();
            }
            close = true;
        }

        if ui.button("Quit").clicked() {
            if let Some(outer) = root_outer {
                let on_top = self.cfg.lock().map(|c| c.always_on_top).unwrap_or(true);
                self.save_window_fields(outer.min.x, outer.min.y, on_top);
            }
            self.quit_pending = true;
            close = true;
        }
        close
    }

    fn show_popup_menu(&mut self, ctx: &egui::Context, taskbar_mode: bool) {
        let Some(menu) = self.menu.as_ref() else {
            return;
        };
        let size = menu.size;
        let pos = menu.position();
        let root_outer = ctx.input(|i| i.viewport().outer_rect);

        let builder = egui::ViewportBuilder::default()
            .with_title("AI Usage menu")
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_active(true)
            .with_position(pos)
            .with_inner_size(size);

        let menu_id = egui::ViewportId::from_hash_of("ai-usage-popup-menu");
        let (close, measured) = ctx.show_viewport_immediate(menu_id, builder, |ctx, _class| {
            let frame = egui::Frame::new()
                .fill(Color32::from_rgb(27, 27, 32))
                .stroke(egui::Stroke::new(1.0, Color32::from_gray(70)))
                .corner_radius(6.0)
                .inner_margin(6.0);
            let mut close = false;
            let mut measured = None;
            egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
                let inner = ui.vertical(|ui| self.menu_items(ui, ctx, taskbar_mode, root_outer));
                close = inner.inner;
                // Content plus the frame's margin and stroke on both sides.
                measured = Some(inner.response.rect.size() + Vec2::splat(2.0 * 7.0));
            });

            let (focused, escape) = ctx.input(|i| {
                (i.viewport().focused, i.key_pressed(egui::Key::Escape))
            });
            if let Some(m) = self.menu.as_mut() {
                if focused == Some(true) {
                    m.seen_focus = true;
                } else if m.seen_focus && focused == Some(false) {
                    close = true;
                }
            }
            (close || escape, measured)
        });

        if close {
            self.menu = None;
            return;
        }
        if let (Some(m), Some(measured)) = (self.menu.as_mut(), measured) {
            let measured = measured.ceil();
            if (measured - m.size).length() > 1.0 {
                m.size = measured;
                ctx.request_repaint();
            }
        }
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

        let (horizontal, taskbar_mode) = self
            .cfg
            .lock()
            .map(|c| (is_horizontal(&c), c.taskbar_mode))
            .unwrap_or((false, false));

        let panel_frame = if horizontal {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(18, 18, 22, 245))
                .stroke(egui::Stroke::new(1.0, Color32::from_gray(55)))
                .corner_radius(6.0)
                .inner_margin(4.0)
        } else {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(18, 18, 22, 245))
                .stroke(egui::Stroke::new(1.0, Color32::from_gray(55)))
                .corner_radius(8.0)
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

            if horizontal {
                // The strip is too short to hold an inline menu (it would be
                // clipped), so open it in its own popup window instead.
                if bg.secondary_clicked() {
                    let pointer = ctx.input(|i| i.pointer.interact_pos());
                    if let (Some(outer), Some(p)) =
                        (ctx.input(|i| i.viewport().outer_rect), pointer)
                    {
                        self.menu = Some(PopupMenu {
                            anchor_x: outer.min.x + p.x,
                            strip: outer,
                            size: MENU_SIZE_ESTIMATE,
                            seen_focus: false,
                        });
                    }
                } else if bg.clicked() {
                    self.menu = None;
                }
            } else {
                bg.context_menu(|ui| {
                    let root_outer = ctx.input(|i| i.viewport().outer_rect);
                    if self.menu_items(ui, ctx, taskbar_mode, root_outer) {
                        ui.close();
                    }
                });
            }

            let (claude, cursor, refreshing) = {
                let s = self.shared.lock().expect("shared lock");
                (s.claude.clone(), s.cursor.clone(), s.refreshing)
            };

            let now = Utc::now();

            if horizontal {
                compact_rect =
                    compact::draw_compact(ui, claude.as_ref(), cursor.as_ref(), now, refreshing);
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
                            if refreshing {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new("⟳").color(Color32::GRAY).small(),
                                        );
                                    },
                                );
                            }
                        });
                    })
                    .response
                    .rect;
                content_bottom = content_rect.max.y;
            }
        });

        if !horizontal {
            self.menu = None;
        }
        self.show_popup_menu(ctx, taskbar_mode);

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
    }
}
