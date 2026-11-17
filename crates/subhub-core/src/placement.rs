//! Pure placement decisions for the taskbar display and hover cards: which monitors' taskbars
//! get a display, where a hover card goes, and stepping past other apps' taskbar widgets. The app supplies the monitors and rectangles
//! (Win32); nothing here touches the OS.
use crate::settings::TaskbarMonitors;

/// A rectangle in one unit (logical or physical px, the caller decides).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub fn width(&self) -> f32 {
        self.right - self.left
    }
    pub fn center_y(&self) -> f32 {
        (self.top + self.bottom) / 2.0
    }
}

/// A connected monitor as the app sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    /// Stable display id (monitor device path), saved in settings.
    pub id: String,
    /// The monitor with the main taskbar (and the tray icon).
    pub is_main: bool,
    /// Windows shows a horizontal taskbar on it.
    pub has_taskbar: bool,
}

/// Monitor ids that get a taskbar display, in the given monitor order. Chosen monitors that
/// are missing are skipped; when none of them is there, the main taskbar is used until one
/// comes back (owner 2026-11-07). Empty only when no taskbar exists at all.
pub fn display_monitors(choice: &TaskbarMonitors, monitors: &[MonitorInfo]) -> Vec<String> {
    let with_bar = || monitors.iter().filter(|m| m.has_taskbar);
    let main = || {
        with_bar()
            .filter(|m| m.is_main)
            .map(|m| m.id.clone())
            .collect::<Vec<_>>()
    };
    match choice {
        TaskbarMonitors::Main => main(),
        TaskbarMonitors::All => with_bar().map(|m| m.id.clone()).collect(),
        TaskbarMonitors::Chosen(ids) => {
            let picked: Vec<String> = with_bar()
                .filter(|m| ids.contains(&m.id))
                .map(|m| m.id.clone())
                .collect();
            if picked.is_empty() { main() } else { picked }
        }
    }
}

/// Top-left of a hover card of `w` x `h` next to `anchor` (the tray icon or a badge): centred
/// on it, on the desktop side of the taskbar (above a bottom taskbar, below a top one), `gap`
/// away from the anchor and at least `margin` inside the monitor's work area on the other sides.
pub fn card_origin(anchor: Rect, w: f32, h: f32, work: Rect, margin: f32, gap: f32) -> (f32, f32) {
    let cx = (anchor.left + anchor.right) / 2.0;
    let min_x = work.left + margin;
    let max_x = (work.right - margin - w).max(min_x);
    let x = (cx - w / 2.0).clamp(min_x, max_x);
    let y = if anchor.center_y() >= work.center_y() {
        // Bottom taskbar: above the anchor (the work area ends where the taskbar starts).
        (anchor.top.min(work.bottom) - gap - h).max(work.top + margin)
    } else {
        (anchor.bottom.max(work.top) + gap).min((work.bottom - margin - h).max(work.top))
    };
    (x, y)
}

/// Left edge for a `w`-wide strip wanted at `x` that does not overlap any of `taken` (left,
/// right spans, e.g. another app's widget on the taskbar), keeping `gap` from them. It moves
/// further left (`leftward`, the side away from the tray) or right until it fits.
pub fn step_aside(x: f32, w: f32, gap: f32, taken: &[(f32, f32)], leftward: bool) -> f32 {
    let mut x = x;
    // Each pass clears at least one span, so this ends within `taken.len()` passes.
    for _ in 0..=taken.len() {
        let hit = taken
            .iter()
            .filter(|(l, r)| r > l && x < r + gap && x + w > l - gap)
            .copied()
            .reduce(|a, b| if leftward == (b.0 < a.0) { b } else { a });
        match hit {
            Some((l, _)) if leftward => x = l - gap - w,
            Some((_, r)) => x = r + gap,
            None => break,
        }
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_aside_passes_widgets_next_to_the_tray() {
        // Free spot: stays.
        assert_eq!(step_aside(100.0, 50.0, 8.0, &[(300.0, 400.0)], true), 100.0);
        // A widget left of the tray (WirelessBattery): moves left of it.
        assert_eq!(step_aside(1750.0, 416.0, 8.0, &[(2039.0, 2172.0)], true), 1615.0);
        // Two adjacent widgets: clears both.
        let two = [(2039.0, 2172.0), (1900.0, 2030.0)];
        assert_eq!(step_aside(1750.0, 416.0, 8.0, &two, true), 1476.0);
        // Rightward from the widgets button.
        assert_eq!(step_aside(60.0, 100.0, 8.0, &[(100.0, 200.0)], false), 208.0);
        // Empty spans are ignored.
        assert_eq!(step_aside(10.0, 50.0, 8.0, &[(20.0, 20.0)], true), 10.0);
    }

    fn mon(id: &str, is_main: bool, has_taskbar: bool) -> MonitorInfo {
        MonitorInfo {
            id: id.into(),
            is_main,
            has_taskbar,
        }
    }

    fn r(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn main_all_and_chosen() {
        let ms = [
            mon("a", false, true),
            mon("b", true, true),
            mon("c", false, false),
        ];
        assert_eq!(display_monitors(&TaskbarMonitors::Main, &ms), ["b"]);
        assert_eq!(display_monitors(&TaskbarMonitors::All, &ms), ["a", "b"]);
        let chosen = TaskbarMonitors::Chosen(vec!["a".into(), "gone".into()]);
        assert_eq!(display_monitors(&chosen, &ms), ["a"]);
    }

    #[test]
    fn missing_chosen_monitor_falls_back_to_main() {
        let ms = [mon("main", true, true)];
        let chosen = TaskbarMonitors::Chosen(vec!["docked".into()]);
        assert_eq!(display_monitors(&chosen, &ms), ["main"]);
        // A chosen monitor without a taskbar (Windows shows it only on the main one) counts as
        // missing too.
        let ms = [mon("main", true, true), mon("docked", false, false)];
        assert_eq!(display_monitors(&chosen, &ms), ["main"]);
    }

    #[test]
    fn no_taskbar_at_all_means_no_display() {
        assert!(display_monitors(&TaskbarMonitors::All, &[mon("a", true, false)]).is_empty());
    }

    #[test]
    fn card_sits_above_a_bottom_taskbar_centred_on_the_anchor() {
        let work = r(0.0, 0.0, 1920.0, 1032.0);
        let icon = r(1700.0, 1040.0, 1724.0, 1072.0);
        assert_eq!(
            card_origin(icon, 200.0, 100.0, work, 12.0, 8.0),
            (1612.0, 924.0)
        );
    }

    #[test]
    fn card_is_kept_inside_the_work_area() {
        let work = r(0.0, 0.0, 1920.0, 1032.0);
        // Near the right edge: shifted left to keep the 12 px margin.
        let icon = r(1890.0, 1040.0, 1914.0, 1072.0);
        assert_eq!(card_origin(icon, 300.0, 100.0, work, 12.0, 8.0).0, 1608.0);
        // Taller than the space above: clamped to the top margin.
        assert_eq!(card_origin(icon, 300.0, 2000.0, work, 12.0, 8.0).1, 12.0);
    }

    #[test]
    fn card_goes_below_a_top_taskbar() {
        let work = r(0.0, 48.0, 1920.0, 1080.0);
        let icon = r(1700.0, 8.0, 1724.0, 40.0);
        assert_eq!(card_origin(icon, 200.0, 100.0, work, 12.0, 8.0).1, 56.0);
    }

    #[test]
    fn second_monitor_left_of_the_main_one() {
        // Negative coordinates: a monitor placed left of the primary.
        let work = r(-2560.0, 0.0, 0.0, 1392.0);
        let badge = r(-300.0, 1394.0, -170.0, 1440.0);
        let (x, y) = card_origin(badge, 260.0, 120.0, work, 12.0, 8.0);
        assert_eq!((x, y), (-365.0, 1264.0));
    }
}
