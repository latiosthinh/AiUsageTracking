# AI Usage Widget

A small Windows-only desktop widget that shows live usage meters for **Claude** and **Cursor** in a borderless, always-on-top window.

## What it shows

For each provider you get plan name (when known), usage bars with percent and reset countdowns, and status when sign-in or fetch fails. Cursor usage (included % and reset time) matches the numbers shown in the Cursor CLI and dashboard **included usage** panel. Data is refreshed on a background thread on a configurable interval (minimum 30 seconds).

When a fetch fails (e.g. HTTP 429) the widget keeps showing the last good numbers with an amber ⚠ "showing data from HH:MM", also across restarts (stored in `%APPDATA%\ai-usage-widget\last_good.json`).

Only one copy of the widget runs at a time; launching it again does nothing while it's already running.

## Build and run

```powershell
cargo build --release
.\target\release\ai-usage-widget.exe
```

Debug builds show a console window; release builds run without one. To print usage once to the terminal (debug builds keep a console for this path):

```powershell
cargo run -- --once
```

## Config file

Location: `%APPDATA%\ai-usage-widget\config.json` (created on first run).

| Field | Purpose |
|-------|---------|
| `cursor_session_token` | Session cookie for Cursor usage when the IDE DB is unavailable |
| `window_x`, `window_y` | Last window position |
| `always_on_top` | Keep the widget above other windows |
| `poll_seconds` | Refresh interval (clamped to at least 30) |
| `layout` | `"vertical"` (default), `"horizontal"` (two-row strip), or `"icons"` (one row: Claude crab and Cursor logos with vertical usage bars; each bar shows its percentage above it; hover for details; in taskbar mode the strip matches the taskbar height). `"rings"` is still accepted. |
| `taskbar_mode` | `true` docks the strip over the taskbar (implies horizontal layout and always on top) |
| `transparent_background` | `true` removes the panel background and border; also in the tray menu |

### Cursor token (optional)

If Cursor IDE is not signed in on this machine, copy the **`WorkosCursorSessionToken`** cookie value from cursor.com in DevTools → Application → Cookies and paste it into `cursor_session_token` in the config file. The app reloads config on each poll, so you do not need to restart after pasting.

## Controls

- **Drag** anywhere on the panel to move the window.
- Right-click on the widget does nothing; all options are in the **tray icon** menu.
- **Tray icon**: click the AI Usage icon in the notification area (it may be under the **^** hidden-icons arrow; drag it onto the taskbar to keep it visible) for Refresh now, Always on top, **Layout** (Vertical / Horizontal / Icons), **Taskbar mode**, Transparent background, Open config file (Notepad), and Quit.
- Hover a meter, bar, or icon for details. Tooltips appear next to the widget.
- Position and always-on-top preference are saved automatically.

## Taskbar mode

Windows 11 has no supported way to embed apps inside the taskbar (deskbands were removed), so the widget docks a compact strip on top of the taskbar instead. Use `layout` `"horizontal"` or `"icons"` in config (or pick Horizontal or Icons under the tray **Layout** submenu). `"rings"` is still accepted.

- It appears to the left of the tray area.
- Drag it sideways; it snaps back to the taskbar's vertical centre.
- It stays above the taskbar and hides while a fullscreen app or game is running.

In the compact strip, each meter starts with its local reset time (`14:30` for today or within 24 hours, `Fri 09:00` for later); hover a bar for the full meter name and countdown.

Limitations: primary taskbar only; vertical (left/right) taskbars and auto-hide taskbars are not snapped.

## Portable version

Build the portable zip:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-portable.ps1
```

Output: `dist\AI-Usage-Widget-<version>-portable.zip` (version is read from `Cargo.toml`).

Unzip anywhere and run `ai-usage-widget.exe`. No installer, no administrator rights, and no Visual C++ Redistributable (the CRT is statically linked via `.cargo/config.toml`).

Configuration still lives at `%APPDATA%\ai-usage-widget\config.json`. To remove the app, delete the folder you unzipped into (and that config folder if you want).

Optional autostart: place a shortcut to the exe in `shell:startup`.
