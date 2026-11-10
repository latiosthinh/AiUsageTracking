#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PxRect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskbarInfo {
    pub rect: PxRect,
    pub edge: Edge,
    pub tray_left: i32,
}

/// Primary taskbar info, or None if it can't be found.
pub fn taskbar_info() -> Option<TaskbarInfo> {
    taskbar_info_impl()
}

/// Top-level window of THIS process titled "AI Usage".
pub fn find_own_window() -> Option<isize> {
    find_own_window_impl()
}

/// Re-assert topmost z-order without moving, sizing or activating.
pub fn keep_on_top(hwnd: isize) {
    keep_on_top_impl(hwnd);
}

/// Show (without activating) or hide the window, asynchronously.
pub fn set_visible(hwnd: isize, visible: bool) {
    set_visible_impl(hwnd, visible);
}

/// True while a fullscreen app / D3D game / presentation is in the foreground.
pub fn fullscreen_app_active() -> bool {
    fullscreen_app_active_impl()
}

/// Pure: top-left (physical px) for a widget of `size` = (w, h) docked on a horizontal taskbar.
pub fn dock_position(
    info: &TaskbarInfo,
    size: (i32, i32),
    preferred_x: Option<i32>,
) -> Option<(i32, i32)> {
    let (w, h) = size;
    let r = info.rect;

    if matches!(info.edge, Edge::Left | Edge::Right) {
        return None;
    }

    let y = r.top + (r.height() - h) / 2;

    let mut lo = r.left + 4;
    let mut hi = info.tray_left - w - 4;
    if hi < lo {
        lo = r.left;
        hi = (r.left + r.width() - w).max(r.left);
    }

    let x = preferred_x
        .unwrap_or(info.tray_left - w - 8)
        .clamp(lo, hi);

    Some((x, y))
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn taskbar_info_impl() -> Option<TaskbarInfo> {
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct AppBarData {
        cb_size: u32,
        h_wnd: isize,
        u_callback_message: u32,
        u_edge: u32,
        rc: Rect,
        l_param: isize,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHAppBarMessage(dw_message: u32, p_data: *mut AppBarData) -> isize;
    }

    const ABM_GETTASKBARPOS: u32 = 5;

    let mut abd = AppBarData {
        cb_size: std::mem::size_of::<AppBarData>() as u32,
        h_wnd: 0,
        u_callback_message: 0,
        u_edge: 0,
        rc: Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        l_param: 0,
    };

    // SAFETY: `abd` is a valid, fully initialized APPBARDATA with correct cbSize.
    let ok = unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd) };
    if ok == 0 {
        return None;
    }

    let edge = match abd.u_edge {
        0 => Edge::Left,
        1 => Edge::Top,
        2 => Edge::Right,
        _ => Edge::Bottom,
    };

    let rect = PxRect {
        left: abd.rc.left,
        top: abd.rc.top,
        right: abd.rc.right,
        bottom: abd.rc.bottom,
    };

    let tray_left = tray_notify_left(rect.right);

    Some(TaskbarInfo {
        rect,
        edge,
        tray_left,
    })
}

#[cfg(windows)]
fn tray_notify_left(fallback: i32) -> i32 {
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(lp_class_name: *const u16, lp_window_name: *const u16) -> isize;
        fn FindWindowExW(
            h_wnd_parent: isize,
            h_wnd_child_after: isize,
            lpsz_class: *const u16,
            lpsz_window: *const u16,
        ) -> isize;
        fn GetWindowRect(h_wnd: isize, lp_rect: *mut Rect) -> i32;
    }

    let tray_class = wide("Shell_TrayWnd");
    // SAFETY: `tray_class` is a valid NUL-terminated wide string.
    let tray = unsafe { FindWindowW(tray_class.as_ptr(), std::ptr::null()) };
    if tray == 0 {
        return fallback;
    }

    let notify_class = wide("TrayNotifyWnd");
    // SAFETY: `tray` is a valid HWND; class name is NUL-terminated.
    let notify = unsafe {
        FindWindowExW(
            tray,
            0,
            notify_class.as_ptr(),
            std::ptr::null(),
        )
    };
    if notify == 0 {
        return fallback;
    }

    let mut rc = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `notify` is a valid HWND; `rc` points to writable RECT storage.
    if unsafe { GetWindowRect(notify, &mut rc) } == 0 {
        return fallback;
    }

    if rc.right - rc.left == 0 {
        return fallback;
    }

    rc.left
}

#[cfg(windows)]
mod find_window {
    #[link(name = "user32")]
    extern "system" {
        pub(super) fn EnumWindows(
            lp_enum_func: Option<unsafe extern "system" fn(isize, isize) -> i32>,
            lparam: isize,
        ) -> i32;
        pub(super) fn GetWindowTextW(h_wnd: isize, lp_string: *mut u16, n_max_count: i32) -> i32;
        pub(super) fn GetWindowThreadProcessId(h_wnd: isize, lpdw_process_id: *mut u32) -> u32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub(super) fn GetCurrentProcessId() -> u32;
    }
}

#[cfg(windows)]
const OWN_WINDOW_TITLE: [u16; 8] = [
    0x0041, 0x0049, 0x0020, 0x0055, 0x0073, 0x0061, 0x0067, 0x0065,
];

#[cfg(windows)]
unsafe extern "system" fn find_own_window_enum_cb(hwnd: isize, lparam: isize) -> i32 {
    use find_window::{GetCurrentProcessId, GetWindowTextW, GetWindowThreadProcessId};

    let mut pid = 0u32;
    // SAFETY: `hwnd` is provided by EnumWindows; `pid` is valid stack storage.
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    // SAFETY: GetCurrentProcessId has no pointer parameters.
    if pid != unsafe { GetCurrentProcessId() } {
        return 1;
    }

    let mut buf = [0u16; 256];
    // SAFETY: `hwnd` is valid; `buf` has capacity for n_max_count wide chars.
    let len = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if len as usize != OWN_WINDOW_TITLE.len() {
        return 1;
    }

    if buf[..OWN_WINDOW_TITLE.len()] != OWN_WINDOW_TITLE {
        return 1;
    }

    // SAFETY: `lparam` is a pointer to `*mut isize` passed from the caller.
    let out = unsafe { &mut *(lparam as *mut isize) };
    *out = hwnd;
    0
}

#[cfg(windows)]
fn find_own_window_impl() -> Option<isize> {
    use find_window::EnumWindows;

    let mut found = 0isize;
    // SAFETY: callback stops on match; `found` lives for the duration of the call.
    let _ = unsafe {
        EnumWindows(
            Some(find_own_window_enum_cb),
            (&mut found as *mut isize) as isize,
        )
    };

    if found == 0 {
        None
    } else {
        Some(found)
    }
}

#[cfg(windows)]
fn keep_on_top_impl(hwnd: isize) {
    #[link(name = "user32")]
    extern "system" {
        fn SetWindowPos(
            h_wnd: isize,
            h_wnd_insert_after: isize,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            u_flags: u32,
        ) -> i32;
    }

    const HWND_TOPMOST: isize = -1;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_ASYNCWINDOWPOS: u32 = 0x4000;

    let flags = SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS;
    // SAFETY: `hwnd` is expected to be a valid window handle from the caller.
    unsafe {
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, flags);
    }
}

#[cfg(windows)]
fn set_visible_impl(hwnd: isize, visible: bool) {
    #[link(name = "user32")]
    extern "system" {
        fn ShowWindowAsync(h_wnd: isize, n_cmd_show: i32) -> i32;
    }

    const SW_HIDE: i32 = 0;
    const SW_SHOWNOACTIVATE: i32 = 4;
    let cmd = if visible { SW_SHOWNOACTIVATE } else { SW_HIDE };
    // SAFETY: `hwnd` is expected to be a valid window handle from the caller.
    unsafe {
        ShowWindowAsync(hwnd, cmd);
    }
}

#[cfg(windows)]
fn fullscreen_app_active_impl() -> bool {
    #[link(name = "shell32")]
    extern "system" {
        fn SHQueryUserNotificationState(pquns: *mut i32) -> i32;
    }

    const QUNS_BUSY: i32 = 2;
    const QUNS_RUNNING_D3D_FULL_SCREEN: i32 = 3;
    const QUNS_PRESENTATION_MODE: i32 = 4;

    let mut state = 0i32;
    // SAFETY: `state` is valid writable storage for the notification state enum.
    let hr = unsafe { SHQueryUserNotificationState(&mut state) };
    if hr != 0 {
        return false;
    }

    matches!(
        state,
        QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE
    )
}

#[cfg(not(windows))]
fn taskbar_info_impl() -> Option<TaskbarInfo> {
    None
}

#[cfg(not(windows))]
fn find_own_window_impl() -> Option<isize> {
    None
}

#[cfg(not(windows))]
fn keep_on_top_impl(_hwnd: isize) {}

#[cfg(not(windows))]
fn set_visible_impl(_hwnd: isize, _visible: bool) {}

#[cfg(not(windows))]
fn fullscreen_app_active_impl() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bottom_taskbar(tray_left: i32) -> TaskbarInfo {
        TaskbarInfo {
            rect: PxRect {
                left: 0,
                top: 1032,
                right: 1920,
                bottom: 1080,
            },
            edge: Edge::Bottom,
            tray_left,
        }
    }

    #[test]
    fn dock_default_left_of_tray() {
        let info = bottom_taskbar(1700);
        assert_eq!(dock_position(&info, (300, 40), None), Some((1392, 1036)));
    }

    #[test]
    fn dock_keeps_preferred_x() {
        let info = bottom_taskbar(1700);
        assert_eq!(dock_position(&info, (300, 40), Some(500)), Some((500, 1036)));
    }

    #[test]
    fn dock_clamps_into_free_area() {
        let info = bottom_taskbar(1700);
        assert_eq!(
            dock_position(&info, (300, 40), Some(1800)),
            Some((1396, 1036))
        );
        assert_eq!(dock_position(&info, (300, 40), Some(-50)), Some((4, 1036)));
    }

    #[test]
    fn dock_vertical_taskbar_is_none() {
        let info = TaskbarInfo {
            rect: PxRect {
                left: 0,
                top: 1032,
                right: 1920,
                bottom: 1080,
            },
            edge: Edge::Left,
            tray_left: 1700,
        };
        assert_eq!(dock_position(&info, (300, 40), None), None);
    }

    #[test]
    fn dock_top_taskbar() {
        let info = TaskbarInfo {
            rect: PxRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 48,
            },
            edge: Edge::Top,
            tray_left: 1700,
        };
        assert_eq!(dock_position(&info, (300, 40), None), Some((1392, 4)));
    }

    #[test]
    fn dock_tiny_free_area_falls_back() {
        let info = bottom_taskbar(200);
        assert_eq!(dock_position(&info, (300, 40), None), Some((0, 1036)));
    }
}
