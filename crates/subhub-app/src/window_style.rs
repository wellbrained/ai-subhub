//! Native Windows 11 window chrome via DWM: rounded corners, border color and shadow.
//! Drawing our own rounded border on a transparent window left the system's square 1 px border
//! visible around it; letting DWM round and outline the window gives clean, anti-aliased edges.
use gpui::{Hsla, Rgba, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};

fn hwnd(window: &Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(h) = handle.as_raw() else {
        return None;
    };
    Some(HWND(h.hwnd.get() as *mut _))
}

/// The cursor's screen y (physical px).
pub fn cursor_screen_y() -> Option<i32> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    Some(p.y)
}

/// Whether the left mouse button is held down right now (also outside our windows).
pub fn left_button_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 }
}

/// Top of the work area of the window's monitor (physical px).
pub fn work_area_top(window: &Window) -> Option<i32> {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    let hwnd = hwnd(window)?;
    unsafe {
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(mon, &mut info).as_bool() {
            return None;
        }
        Some(info.rcWork.top)
    }
}

/// Resizes through GPUI (so its surface follows) and keeps the outer bottom edge in place.
pub fn resize_keep_bottom(window: &mut Window, cx: &mut gpui::App, height: f32) {
    if let Some(b) = outer_bottom(window) {
        resize_to_bottom(window, cx, height, b);
    }
}

/// Resizes to a client `height` (logical px) with the outer bottom edge at `bottom` (screen px).
/// The target top is computed up front from the frame size (outer minus client, constant), and
/// the window is moved there without resizing; GPUI's own `resize` (keeps the top, applied when
/// GPUI gets to it) then grows or shrinks it downwards onto `bottom`. Reading the rect after
/// `resize` instead raced GPUI and let the bottom edge wander (top-edge drag, 2026-11-07).
pub fn resize_to_bottom(window: &mut Window, cx: &mut gpui::App, height: f32, bottom: i32) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect};
    let Some(hwnd) = hwnd(window) else { return };
    let (mut outer, mut client) = (RECT::default(), RECT::default());
    unsafe {
        if GetWindowRect(hwnd, &mut outer).is_err() || GetClientRect(hwnd, &mut client).is_err() {
            return;
        }
    }
    let frame = (outer.bottom - outer.top) - (client.bottom - client.top);
    let new_outer = frame + (height * window.scale_factor()).round() as i32;
    let (x, y) = (outer.left, bottom - new_outer);
    if y != outer.top {
        move_outer(window, x, y);
    }
    let w = window.viewport_size().width;
    window.resize(gpui::size(w, gpui::px(height)));
    window.defer(cx, |window, _| window.refresh());
}

/// Outer bottom edge of the window in screen pixels.
pub fn outer_bottom(window: &Window) -> Option<i32> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let hwnd = hwnd(window)?;
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r).ok()? };
    Some(r.bottom)
}

/// Moves/resizes so the *client area* (what GPUI draws, and what GPUI's own window bounds mean)
/// is at `b` (logical px), without activating. `SetWindowPos` takes the outer rect, which for
/// GPUI's popup style includes an invisible frame; without correcting for it the window landed
/// ~10 px too high (taskbar display drifting above the taskbar after the first re-place).
pub fn set_bounds(window: &mut Window, b: gpui::Bounds<gpui::Pixels>) {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowRect, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
    };
    let Some(hwnd) = hwnd(window) else { return };
    let s = window.scale_factor();
    let r = |v: gpui::Pixels| (v.as_f32() * s).round() as i32;
    unsafe {
        let (mut outer, mut client) = (RECT::default(), RECT::default());
        let mut origin = POINT::default();
        if GetWindowRect(hwnd, &mut outer).is_err() || GetClientRect(hwnd, &mut client).is_err() {
            return;
        }
        let _ = ClientToScreen(hwnd, &mut origin);
        // Frame thickness on each side.
        let (left, top) = (origin.x - outer.left, origin.y - outer.top);
        let extra_w = (outer.right - outer.left) - (client.right - client.left);
        let extra_h = (outer.bottom - outer.top) - (client.bottom - client.top);
        let _ = SetWindowPos(
            hwnd,
            None,
            r(b.origin.x) - left,
            r(b.origin.y) - top,
            r(b.size.width) + extra_w,
            r(b.size.height) + extra_h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// True when the foreground window covers its whole monitor (fullscreen video, game,
/// presentation) or Windows reports a fullscreen/presentation state. The desktop and the
/// taskbar itself never count.
pub fn fullscreen_app_in_front() -> bool {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows::Win32::UI::Shell::{
        QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
        SHQueryUserNotificationState,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowRect,
    };
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState()
            && (state == QUNS_BUSY
                || state == QUNS_RUNNING_D3D_FULL_SCREEN
                || state == QUNS_PRESENTATION_MODE)
        {
            return true;
        }
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        let mut buf = [0u16; 64];
        let n = GetClassNameW(fg, &mut buf) as usize;
        let class = String::from_utf16_lossy(&buf[..n]);
        if matches!(
            class.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return false;
        }
        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(mon, &mut info).as_bool() {
            return false;
        }
        let m = info.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

/// Shows or hides without activating (used to step aside for fullscreen apps).
pub fn set_visible(window: &mut Window, visible: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindowVisible, SW_HIDE, SW_SHOWNOACTIVATE, ShowWindow,
    };
    let Some(hwnd) = hwnd(window) else { return };
    unsafe {
        if IsWindowVisible(hwnd).as_bool() != visible {
            let _ = ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
    }
}

/// Demo `--demo-hidden`: turns a window that was created hidden into a tool window (no taskbar
/// or alt-tab entry), puts it far off every monitor at `x, y` (physical px; GPUI itself replaces
/// off-display bounds with on-screen defaults), shows it there without activating it and sizes
/// its client area to `w x h` logical px. `PrintWindow` still renders its content.
///
/// Like GPUI's own `resize`, the Win32 calls run in a later foreground task: their `WM_SIZE`
/// must reach GPUI while no window update is running, or the new size is lost.
pub fn show_off_screen(window: &Window, cx: &mut gpui::App, x: i32, y: i32, w: f32, h: f32) {
    let Some(hwnd) = hwnd(window) else { return };
    let raw = hwnd.0 as isize;
    let s = window.scale_factor();
    cx.spawn(async move |_| unsafe {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetClientRect, GetWindowLongPtrW, GetWindowRect, IsWindowVisible,
            SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW,
            SetWindowPos, ShowWindow, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
        };
        let hwnd = HWND(raw as *mut _);
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let ex = (ex | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
        // Move while still hidden, show there, then size.
        let flags = SWP_NOACTIVATE | SWP_NOZORDER;
        let _ = SetWindowPos(hwnd, None, x, y, 0, 0, flags | SWP_NOSIZE);
        if !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        let (mut outer, mut client) = (RECT::default(), RECT::default());
        if GetWindowRect(hwnd, &mut outer).is_ok() && GetClientRect(hwnd, &mut client).is_ok() {
            let extra_w = (outer.right - outer.left) - (client.right - client.left);
            let extra_h = (outer.bottom - outer.top) - (client.bottom - client.top);
            let (pw, ph) = ((w * s).round() as i32, (h * s).round() as i32);
            let _ = SetWindowPos(hwnd, None, x, y, pw + extra_w, ph + extra_h, flags);
        }
    })
    .detach();
}

/// Puts the window back on top of other topmost windows (the taskbar) without activating it.
pub fn raise_topmost(window: &mut Window) {
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
    };
    let Some(hwnd) = hwnd(window) else { return };
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Removes the Windows 11 frame DWM draws around popup windows (thin border, rounded
/// corners), so a transparent window shows only its own content (taskbar display).
pub fn no_frame(window: &Window) {
    use windows::Win32::Graphics::Dwm::DWMWCP_DONOTROUND;
    let Some(hwnd) = hwnd(window) else { return };
    let corner = DWMWCP_DONOTROUND;
    let none: u32 = 0xFFFF_FFFE; // DWMWA_COLOR_NONE
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            size_of_val(&corner) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &none as *const _ as *const _,
            size_of::<u32>() as u32,
        );
        // GPUI's popup style includes WS_CAPTION, so DWM also drew a ~8 px drop shadow on both
        // sides (measured 2026-11-06: the taskbar darkened from ~100 to ~76 at the edges), which
        // made the display look like it sat on top of the taskbar. Disable non-client rendering.
        let disabled: u32 = 1; // DWMNCRP_DISABLED
        let _ = DwmSetWindowAttribute(
            hwnd,
            windows::Win32::Graphics::Dwm::DWMWA_NCRENDERING_POLICY,
            &disabled as *const _ as *const _,
            size_of::<u32>() as u32,
        );
    }
}

/// Asks DWM for rounded corners and a border in `border` (no effect before Windows 11).
pub fn round(window: &Window, border: Hsla) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(h) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(h.hwnd.get() as *mut _);
    let corner = DWMWCP_ROUND;
    let c = Rgba::from(border);
    // COLORREF is 0x00BBGGRR.
    let colorref: u32 =
        ((c.b * 255.0) as u32) << 16 | ((c.g * 255.0) as u32) << 8 | (c.r * 255.0) as u32;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const _,
            size_of_val(&corner) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &colorref as *const _ as *const _,
            size_of::<u32>() as u32,
        );
    }
}

/// Plans a client area of `width` x `height` logical px (height cut to the monitor work area minus
/// a margin), centred: the logical client size to give GPUI and the outer top-left in physical
/// px. GPUI must do the resizing (its viewport follows its own `resize`), then `move_to`.
pub fn fit_plan(window: &Window, width: f32, height: f32) -> Option<((f32, f32), (i32, i32))> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect};
    let hwnd = hwnd(window)?;
    let s = window.scale_factor();
    unsafe {
        let (mut outer, mut client) = (RECT::default(), RECT::default());
        GetWindowRect(hwnd, &mut outer).ok()?;
        GetClientRect(hwnd, &mut client).ok()?;
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if !GetMonitorInfoW(mon, &mut info).as_bool() {
            return None;
        }
        let work = info.rcWork;
        let extra_w = (outer.right - outer.left) - (client.right - client.left);
        let extra_h = (outer.bottom - outer.top) - (client.bottom - client.top);
        let margin = (24.0 * s) as i32;
        let w = ((width * s).round() as i32 + extra_w).min(work.right - work.left - margin);
        let h = ((height * s).round() as i32 + extra_h).min(work.bottom - work.top - margin);
        let x = work.left + (work.right - work.left - w) / 2;
        let y = work.top + (work.bottom - work.top - h) / 2;
        Some((((w - extra_w) as f32 / s, (h - extra_h) as f32 / s), (x, y)))
    }
}

/// Top-left of the client area in physical screen px.
pub fn client_origin(window: &Window) -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    let hwnd = hwnd(window)?;
    let mut p = POINT::default();
    unsafe { ClientToScreen(hwnd, &mut p).ok().ok()? };
    Some((p.x, p.y))
}

/// Puts the client area at `x, y` with size `w x h` (all physical px), topmost, without
/// activating. Done twice: moving onto a monitor with another scale makes Windows resize the
/// window (WM_DPICHANGED) during the first call.
pub fn place_client_physical(window: &Window, x: i32, y: i32, w: i32, h: i32) {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowRect, HWND_TOPMOST, SWP_NOACTIVATE, SetWindowPos,
    };
    let Some(hwnd) = hwnd(window) else { return };
    for _ in 0..2 {
        unsafe {
            let (mut outer, mut client) = (RECT::default(), RECT::default());
            let mut origin = POINT::default();
            if GetWindowRect(hwnd, &mut outer).is_err() || GetClientRect(hwnd, &mut client).is_err()
            {
                return;
            }
            let _ = ClientToScreen(hwnd, &mut origin);
            let (left, top) = (origin.x - outer.left, origin.y - outer.top);
            let extra_w = (outer.right - outer.left) - (client.right - client.left);
            let extra_h = (outer.bottom - outer.top) - (client.bottom - client.top);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                x - left,
                y - top,
                w + extra_w,
                h + extra_h,
                SWP_NOACTIVATE,
            );
        }
    }
}

/// Moves the window (outer top-left, physical px) without resizing or activating.
pub fn move_outer(window: &Window, x: i32, y: i32) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    };
    if let Some(hwnd) = hwnd(window) {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

/// Renders the window's client area to the clipboard as a bitmap. Returns its size in pixels.
pub fn copy_client_to_clipboard(window: &Window) -> Option<(i32, i32)> {
    use windows::Win32::Foundation::{HANDLE, RECT};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, ReleaseDC,
        SelectObject,
    };
    use windows::Win32::Storage::Xps::PrintWindow;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
    let hwnd = hwnd(window)?;
    unsafe {
        let mut r = RECT::default();
        GetClientRect(hwnd, &mut r).ok()?;
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        if w <= 0 || h <= 0 {
            return None;
        }
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bmp.into());
        // PW_CLIENTONLY | PW_RENDERFULLCONTENT: also captures DirectComposition content.
        let drawn = PrintWindow(
            hwnd,
            dc,
            windows::Win32::Storage::Xps::PRINT_WINDOW_FLAGS(3),
        );
        SelectObject(dc, old);
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        let mut ok = false;
        if drawn.as_bool() && OpenClipboard(Some(hwnd)).is_ok() {
            let _ = EmptyClipboard();
            // CF_BITMAP = 2; on success the clipboard owns the bitmap.
            ok = SetClipboardData(2, Some(HANDLE(bmp.0))).is_ok();
            let _ = CloseClipboard();
        }
        if !ok {
            let _ = DeleteObject(bmp.into());
            return None;
        }
        Some((w, h))
    }
}
