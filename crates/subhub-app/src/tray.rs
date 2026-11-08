//! Minimal Windows tray icon on Shell_NotifyIconW. Runs on GPUI's main thread: GPUI's
//! `GetMessageW(None)` loop dispatches our hidden window's messages, so no second loop.
use std::sync::OnceLock;

use futures::channel::mpsc::UnboundedSender;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, HBITMAP,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NIM_SETVERSION, NOTIFY_ICON_DATA_FLAGS, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

const WM_TRAY: u32 = WM_APP + 1;
const NIN_SELECT: u32 = WM_USER;
const NIN_KEYSELECT: u32 = WM_USER + 1;
/// NOTIFYICON_VERSION_4 without NIF_SHOWTIP: the pointer rests on the icon / left it.
const NIN_POPUPOPEN: u32 = WM_USER + 6;
const NIN_POPUPCLOSE: u32 = WM_USER + 7;
/// The user clicked a balloon (billing reminder, run-out nudge): open the popover.
const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;
const ID_OPEN: usize = 1;
const ID_REFRESH: usize = 2;
const ID_SETTINGS: usize = 3;
const ID_QUIT: usize = 4;
const ID_STATS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Toggle,
    Open,
    Refresh,
    Settings,
    Statistics,
    Quit,
    /// Hover card: show (Windows waited its hover time) / hide.
    HoverOpen,
    HoverClose,
    /// The pointer moved over the icon (fallback when no NIN_POPUPOPEN comes).
    HoverMove,
}

static SENDER: OnceLock<UnboundedSender<TrayEvent>> = OnceLock::new();

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    /// Which tray mark is shown: for a dark taskbar or a light one.
    dark: bool,
    /// No notify icon (demo `--demo-hidden`): tips and balloons are skipped.
    hidden: bool,
}

/// Tray icon rectangle in physical screen pixels.
pub struct TrayRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Tray {
    /// `rich`: our own hover card instead of Windows' text tooltip (the text is still set, for
    /// screen readers).
    pub fn new(sender: UnboundedSender<TrayEvent>, tip: &str, rich: bool) -> anyhow::Result<Self> {
        SENDER.set(sender).ok();
        unsafe {
            let hinstance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance.into(),
                lpszClassName: w!("AISubHubTray"),
                ..Default::default()
            };
            RegisterClassW(&class);
            // Hidden top-level window (not message-only) so TrackPopupMenu can take focus.
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("AISubHubTray"),
                w!("AISubHubTray"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinstance.into()),
                None,
            )?;
            let dark = crate::taskbar::taskbar_is_dark();
            let icon = logo_icon(dark)?;
            let mut nid = base_nid(hwnd);
            nid.uFlags = NIF_ICON | NIF_MESSAGE | tip_flags(rich);
            nid.uCallbackMessage = WM_TRAY;
            nid.hIcon = icon;
            set_tip(&mut nid, tip);
            // Demo `--demo-hidden` (screenshots without disturbing the desktop): no icon.
            let hidden = crate::demo::hidden();
            if !hidden {
                Shell_NotifyIconW(NIM_ADD, &nid).ok()?;
                nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &nid).ok()?;
            }
            Ok(Self {
                hwnd,
                icon,
                dark,
                hidden,
            })
        }
    }

    /// Updates the tooltip text and whether Windows draws it (`rich` = our hover card instead).
    /// The icon itself is the static logo (never live, ADR 0001).
    pub fn set_tip(&mut self, tip: &str, rich: bool) -> anyhow::Result<()> {
        if self.hidden {
            return Ok(());
        }
        unsafe {
            let mut nid = base_nid(self.hwnd);
            nid.uFlags = tip_flags(rich);
            set_tip(&mut nid, tip);
            Shell_NotifyIconW(NIM_MODIFY, &nid).ok()?;
        }
        Ok(())
    }

    /// Swaps the tray mark when the Windows mode (taskbar) switched between dark and light.
    pub fn sync_theme(&mut self) {
        let dark = crate::taskbar::taskbar_is_dark();
        if dark == self.dark || self.hidden {
            return;
        }
        let Ok(icon) = logo_icon(dark) else { return };
        unsafe {
            let mut nid = base_nid(self.hwnd);
            nid.uFlags = NIF_ICON;
            nid.hIcon = icon;
            if Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool() {
                let _ = DestroyIcon(self.icon);
                self.icon = icon;
                self.dark = dark;
            } else {
                let _ = DestroyIcon(icon);
            }
        }
    }

    /// A balloon / toast notice from the tray icon (billing reminders, run-out nudge); a click on
    /// it opens the popover.
    pub fn balloon(&mut self, title: &str, text: &str) {
        if self.hidden {
            return;
        }
        unsafe {
            let mut nid = base_nid(self.hwnd);
            nid.uFlags = NIF_INFO;
            put_wide(&mut nid.szInfoTitle, title);
            put_wide(&mut nid.szInfo, text);
            nid.dwInfoFlags = NIIF_INFO;
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    /// Where the icon really is, also inside the overflow flyout (hover cards anchor here: the
    /// pointer is on it).
    pub fn icon_rect(&self) -> Option<TrayRect> {
        let id = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            ..Default::default()
        };
        unsafe { Shell_NotifyIconGetRect(&id).ok() }.map(|r: RECT| TrayRect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        })
    }

    /// Popover anchor: the icon, or the notification area when the icon is in the overflow.
    pub fn rect(&self) -> Option<TrayRect> {
        let icon = self.icon_rect();
        // Windows 11 hides new icons in the overflow flyout; its rect is then above the taskbar.
        // Anchor to the visible notification area in that case.
        let taskbar = taskbar_notify_rect();
        match (icon, taskbar) {
            (Some(i), Some(t)) if i.bottom <= t.top || i.top >= t.bottom => Some(t),
            (Some(i), _) => Some(i),
            (None, t) => t,
        }
    }

    /// Monitor DPI scale of the tray icon's location.
    pub fn scale(&self) -> f32 {
        use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
        use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
        let Some(r) = self.rect() else { return 1.0 };
        let pt = POINT {
            x: (r.left + r.right) / 2,
            y: (r.top + r.bottom) / 2,
        };
        unsafe {
            let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let (mut x, mut y) = (96u32, 96u32);
            if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_ok() {
                return x as f32 / 96.0;
            }
        }
        1.0
    }
}

/// Work area (screen minus taskbar) of the monitor at `r`, in physical pixels.
pub fn work_area(r: &TrayRect) -> Option<TrayRect> {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    };
    let pt = POINT {
        x: (r.left + r.right) / 2,
        y: (r.top + r.bottom) / 2,
    };
    unsafe {
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(mon, &mut info).ok().ok()?;
        let w = info.rcWork;
        Some(TrayRect {
            left: w.left,
            top: w.top,
            right: w.right,
            bottom: w.bottom,
        })
    }
}

fn taskbar_notify_rect() -> Option<TrayRect> {
    unsafe {
        let bar = FindWindowW(w!("Shell_TrayWnd"), None).ok()?;
        let notify = FindWindowExW(Some(bar), None, w!("TrayNotifyWnd"), None).unwrap_or(bar);
        let mut r = RECT::default();
        GetWindowRect(notify, &mut r).ok()?;
        Some(TrayRect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        })
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let nid = base_nid(self.hwnd);
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            let _ = DestroyIcon(self.icon);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn base_nid(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    }
}

/// The tip text is always set (screen readers read it). With NIF_SHOWTIP Windows draws it; without
/// it (NOTIFYICON_VERSION_4) Windows sends NIN_POPUPOPEN / NIN_POPUPCLOSE for our own card.
fn tip_flags(rich: bool) -> NOTIFY_ICON_DATA_FLAGS {
    if rich { NIF_TIP } else { NIF_TIP | NIF_SHOWTIP }
}

fn set_tip(nid: &mut NOTIFYICONDATAW, tip: &str) {
    put_wide(&mut nid.szTip, tip);
}

/// Copies `text` into a fixed UTF-16 buffer, cut to fit, NUL-terminated.
fn put_wide(dst: &mut [u16], text: &str) {
    let wide: Vec<u16> = text.encode_utf16().take(dst.len() - 1).collect();
    dst[..wide.len()].copy_from_slice(&wide);
    dst[wide.len()] = 0;
}

fn send(event: TrayEvent) {
    if let Some(s) = SENDER.get() {
        let _ = s.unbounded_send(event);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_TRAY {
        // NOTIFYICON_VERSION_4: LOWORD(lParam) = event. A left click sends WM_LBUTTONUP *and*
        // NIN_SELECT, and a right click WM_RBUTTONUP *and* WM_CONTEXTMENU: handle one of each only.
        match (lparam.0 as u32) & 0xffff {
            NIN_SELECT | NIN_KEYSELECT => send(TrayEvent::Toggle),
            NIN_BALLOONUSERCLICK => send(TrayEvent::Open),
            NIN_POPUPOPEN => send(TrayEvent::HoverOpen),
            NIN_POPUPCLOSE => send(TrayEvent::HoverClose),
            WM_MOUSEMOVE => send(TrayEvent::HoverMove),
            WM_CONTEXTMENU => unsafe { show_menu(hwnd) },
            _ => {}
        }
        return LRESULT(0);
    }
    if msg == WM_COMMAND {
        let ev = match wparam.0 & 0xffff {
            ID_OPEN => Some(TrayEvent::Open),
            ID_REFRESH => Some(TrayEvent::Refresh),
            ID_SETTINGS => Some(TrayEvent::Settings),
            ID_STATS => Some(TrayEvent::Statistics),
            ID_QUIT => Some(TrayEvent::Quit),
            _ => None,
        };
        if let Some(ev) = ev {
            send(ev);
            return LRESULT(0);
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

unsafe fn show_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN, w!("Open AI SubHub"));
        let _ = AppendMenuW(menu, MF_STRING, ID_REFRESH, w!("Refresh usage"));
        let _ = AppendMenuW(menu, MF_STRING, ID_STATS, w!("Statistics"));
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, w!("Settings"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quit"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            Some(0),
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
    }
}

/// Debug: writes rings (composited on a taskbar-dark background, 4x) to a 24-bit BMP.
#[allow(dead_code)]
pub fn dump_rings(path: &str, rings: &[(f32, [u8; 3])]) -> std::io::Result<()> {
    let (n, s) = (32usize, 4usize);
    let (w, h) = (n * s * rings.len(), n * s);
    let row = (w * 3 + 3) & !3;
    let mut data = vec![0u8; row * h];
    for (k, (f, c)) in rings.iter().enumerate() {
        let px = ring_pixels(*f, *c);
        for y in 0..h {
            for x in 0..n * s {
                let i = ((y / s) * n + x / s) * 4;
                let a = px[i + 3] as f32 / 255.0;
                let bg = 0x20 as f32;
                let o = (h - 1 - y) * row + (k * n * s + x) * 3;
                for ch in 0..3 {
                    data[o + ch] = (px[i + ch] as f32 * a + bg * (1.0 - a)) as u8;
                }
            }
        }
    }
    let mut out = Vec::with_capacity(54 + data.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + data.len()) as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    out.extend_from_slice(&data);
    std::fs::write(path, out)
}

/// Draws an anti-aliased ring (track + filled arc from 12 o'clock, clockwise) into a 32x32 icon.
/// The tray mark as a 32x32 icon (Windows scales it for the tray), for a dark or light taskbar.
fn logo_icon(dark: bool) -> anyhow::Result<HICON> {
    let rgba = crate::logo::tray_rgba(32, dark);
    let mut bgra = rgba.clone();
    for i in (0..bgra.len()).step_by(4) {
        bgra.swap(i, i + 2);
    }
    unsafe { icon_from_bgra(&bgra, 32) }
}

#[allow(dead_code)]
fn ring_icon(fraction: f32, rgb: [u8; 3]) -> anyhow::Result<HICON> {
    const N: i32 = 32;
    let px = ring_pixels(fraction, rgb);
    unsafe { icon_from_bgra(&px, N) }
}

fn ring_pixels(fraction: f32, rgb: [u8; 3]) -> Vec<u8> {
    const N: i32 = 32;
    let c = N as f32 / 2.0;
    let (outer, inner) = (15.0f32, 10.0f32);
    let fraction = fraction.clamp(0.0, 1.0);
    let mut px = vec![0u8; (N * N * 4) as usize];
    for y in 0..N {
        for x in 0..N {
            // 4x4 supersampling for smooth edges.
            let (mut track, mut fill) = (0u32, 0u32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let fx = x as f32 + (sx as f32 + 0.5) / 4.0 - c;
                    let fy = y as f32 + (sy as f32 + 0.5) / 4.0 - c;
                    let d = (fx * fx + fy * fy).sqrt();
                    if d <= outer && d >= inner {
                        let angle = (fx.atan2(-fy) / std::f32::consts::TAU).rem_euclid(1.0);
                        if angle <= fraction {
                            fill += 1
                        } else {
                            track += 1
                        }
                    }
                }
            }
            let i = ((y * N + x) * 4) as usize;
            let (fa, ta) = (fill as f32 / 16.0, track as f32 / 16.0 * 0.35);
            let a = fa + ta;
            if a > 0.0 {
                // BGRA, track is light grey.
                let mix = |c: u8, g: u8| ((c as f32 * fa + g as f32 * ta) / a) as u8;
                px[i] = mix(rgb[2], 0xd0);
                px[i + 1] = mix(rgb[1], 0xd0);
                px[i + 2] = mix(rgb[0], 0xd0);
                px[i + 3] = (a.min(1.0) * 255.0) as u8;
            }
        }
    }
    px
}

unsafe fn icon_from_bgra(px: &[u8], n: i32) -> anyhow::Result<HICON> {
    const N: i32 = 32;
    debug_assert_eq!(n, N);
    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: N,
                biHeight: -N,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color: HBITMAP = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u8, px.len());
        let mask = CreateBitmap(N, N, 1, 1, None);
        let info = ICONINFO {
            fIcon: true.into(),
            hbmMask: mask,
            hbmColor: color,
            ..Default::default()
        };
        let icon = CreateIconIndirect(&info);
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        Ok(icon?)
    }
}
