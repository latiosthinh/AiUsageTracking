use std::cell::Cell;
use std::sync::mpsc;

use eframe::egui;
use tray_icon::menu::{
    CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::config::Layout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Refresh,
    ToggleAlwaysOnTop,
    SetLayout(Layout),
    ToggleTaskbarMode,
    ToggleTransparent,
    OpenConfig,
    Quit,
}

const ID_REFRESH: &str = "tray_refresh";
const ID_ON_TOP: &str = "tray_on_top";
const ID_LAYOUT: &str = "tray_layout";
const ID_LAYOUT_VERTICAL: &str = "tray_layout_vertical";
const ID_LAYOUT_HORIZONTAL: &str = "tray_layout_horizontal";
const ID_LAYOUT_ICONS: &str = "tray_layout_icons";
const ID_TASKBAR: &str = "tray_taskbar";
const ID_TRANSPARENT: &str = "tray_transparent";
const ID_OPEN_CONFIG: &str = "tray_open_config";
const ID_QUIT: &str = "tray_quit";

pub struct Tray {
    _icon: TrayIcon,
    rx: mpsc::Receiver<MenuEvent>,
    refresh: MenuItem,
    on_top: CheckMenuItem,
    layout_vertical: CheckMenuItem,
    layout_horizontal: CheckMenuItem,
    layout_icons: CheckMenuItem,
    taskbar: CheckMenuItem,
    transparent: CheckMenuItem,
    open_config: MenuItem,
    quit: MenuItem,
    last_on_top: Cell<Option<bool>>,
    last_layout: Cell<Option<Layout>>,
    last_taskbar: Cell<Option<bool>>,
    last_transparent: Cell<Option<bool>>,
    last_on_top_disabled: Cell<Option<bool>>,
    last_vertical_disabled: Cell<Option<bool>>,
}

impl Tray {
    /// Builds the icon + menu. Errors are returned as a String for logging.
    pub fn new(ctx: &egui::Context) -> Result<Tray, String> {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event| {
            let _ = tx.send(event);
            ctx.request_repaint();
        }));

        let icon_data = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon.png"))
            .map_err(|e| e.to_string())?;
        let icon = Icon::from_rgba(icon_data.rgba, icon_data.width, icon_data.height)
            .map_err(|e| format!("tray icon rgba: {e}"))?;

        let refresh = MenuItem::with_id(ID_REFRESH, "Refresh now", true, None);
        let on_top = CheckMenuItem::with_id(ID_ON_TOP, "Always on top", true, true, None);
        let layout_vertical =
            CheckMenuItem::with_id(ID_LAYOUT_VERTICAL, "Vertical", true, false, None);
        let layout_horizontal =
            CheckMenuItem::with_id(ID_LAYOUT_HORIZONTAL, "Horizontal", true, false, None);
        let layout_icons = CheckMenuItem::with_id(ID_LAYOUT_ICONS, "Icons", true, false, None);
        let layout_submenu = Submenu::with_id(ID_LAYOUT, "Layout", true);
        layout_submenu
            .append(&layout_vertical)
            .map_err(|e| e.to_string())?;
        layout_submenu
            .append(&layout_horizontal)
            .map_err(|e| e.to_string())?;
        layout_submenu
            .append(&layout_icons)
            .map_err(|e| e.to_string())?;
        let taskbar = CheckMenuItem::with_id(ID_TASKBAR, "Taskbar mode", true, false, None);
        let transparent =
            CheckMenuItem::with_id(ID_TRANSPARENT, "Transparent background", true, false, None);
        let open_config = MenuItem::with_id(ID_OPEN_CONFIG, "Open config file", true, None);
        let quit = MenuItem::with_id(ID_QUIT, "Quit", true, None);

        let menu = Menu::new();
        menu.append(&refresh).map_err(|e| e.to_string())?;
        menu.append(&PredefinedMenuItem::separator()).map_err(|e| e.to_string())?;
        menu.append(&on_top).map_err(|e| e.to_string())?;
        menu.append(&layout_submenu).map_err(|e| e.to_string())?;
        menu.append(&taskbar).map_err(|e| e.to_string())?;
        menu.append(&transparent).map_err(|e| e.to_string())?;
        menu.append(&PredefinedMenuItem::separator()).map_err(|e| e.to_string())?;
        menu.append(&open_config).map_err(|e| e.to_string())?;
        menu.append(&quit).map_err(|e| e.to_string())?;

        let _icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("AI Usage")
            .with_icon(icon)
            .with_menu_on_left_click(true)
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Tray {
            _icon,
            rx,
            refresh,
            on_top,
            layout_vertical,
            layout_horizontal,
            layout_icons,
            taskbar,
            transparent,
            open_config,
            quit,
            last_on_top: Cell::new(None),
            last_layout: Cell::new(None),
            last_taskbar: Cell::new(None),
            last_transparent: Cell::new(None),
            last_on_top_disabled: Cell::new(None),
            last_vertical_disabled: Cell::new(None),
        })
    }

    /// Set check marks; `on_top` is disabled while taskbar_mode is on. Layout "Vertical" is
    /// disabled in taskbar mode. Only call setters when values change (Cell caches) to limit
    /// Win32 traffic.
    pub fn sync(
        &self,
        always_on_top: bool,
        layout: Layout,
        taskbar_mode: bool,
        transparent_background: bool,
    ) {
        let on_top_disabled = taskbar_mode;
        if self.last_on_top_disabled.get() != Some(on_top_disabled) {
            self.on_top.set_enabled(!on_top_disabled);
            self.last_on_top_disabled.set(Some(on_top_disabled));
        }
        let vertical_disabled = taskbar_mode;
        if self.last_vertical_disabled.get() != Some(vertical_disabled) {
            self.layout_vertical.set_enabled(!vertical_disabled);
            self.last_vertical_disabled.set(Some(vertical_disabled));
        }
        if self.last_on_top.get() != Some(always_on_top) {
            self.on_top.set_checked(always_on_top);
            self.last_on_top.set(Some(always_on_top));
        }
        if self.last_layout.get() != Some(layout) {
            self.layout_vertical.set_checked(layout == Layout::Vertical);
            self.layout_horizontal.set_checked(layout == Layout::Horizontal);
            self.layout_icons.set_checked(layout == Layout::Icons);
            self.last_layout.set(Some(layout));
        }
        if self.last_taskbar.get() != Some(taskbar_mode) {
            self.taskbar.set_checked(taskbar_mode);
            self.last_taskbar.set(Some(taskbar_mode));
        }
        if self.last_transparent.get() != Some(transparent_background) {
            self.transparent.set_checked(transparent_background);
            self.last_transparent.set(Some(transparent_background));
        }
    }

    /// Drain the channel (try_recv loop) and map each event id to a TrayAction; unknown ids ignored.
    pub fn poll_actions(&self) -> Vec<TrayAction> {
        let mut actions = Vec::new();
        let mut had_event = false;
        while let Ok(event) = self.rx.try_recv() {
            had_event = true;
            let id = event.id();
            let action = if id == self.refresh.id() {
                Some(TrayAction::Refresh)
            } else if id == self.on_top.id() {
                Some(TrayAction::ToggleAlwaysOnTop)
            } else if id == self.layout_vertical.id() {
                Some(TrayAction::SetLayout(Layout::Vertical))
            } else if id == self.layout_horizontal.id() {
                Some(TrayAction::SetLayout(Layout::Horizontal))
            } else if id == self.layout_icons.id() {
                Some(TrayAction::SetLayout(Layout::Icons))
            } else if id == self.taskbar.id() {
                Some(TrayAction::ToggleTaskbarMode)
            } else if id == self.transparent.id() {
                Some(TrayAction::ToggleTransparent)
            } else if id == self.open_config.id() {
                Some(TrayAction::OpenConfig)
            } else if id == self.quit.id() {
                Some(TrayAction::Quit)
            } else {
                None
            };
            if let Some(a) = action {
                actions.push(a);
            }
        }
        if had_event {
            self.last_on_top.set(None);
            self.last_layout.set(None);
            self.last_taskbar.set(None);
            self.last_transparent.set(None);
            self.last_on_top_disabled.set(None);
            self.last_vertical_disabled.set(None);
        }
        actions
    }
}
