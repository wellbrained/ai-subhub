//! Drawn icons (replace text characters used as icons; craft floor). Lucide for UI actions, own
//! marks for providers. All single-colour SVGs tinted with `text_color`.
use gpui::{Hsla, Styled, Svg, px, svg};
use subhub_core::model::ProviderKind;

#[derive(Clone, Copy)]
pub enum Icon {
    Refresh,
    Settings,
    Close,
    Star,
    ArrowUp,
    ArrowDown,
    Plus,
    Info,
    EyeOff,
    Eye,
    Trash,
    Check,
    ExternalLink,
    LogIn,
    Chart,
    Copy,
    Rocket,
    Pencil,
    FolderPlus,
    Warning,
    Terminal,
    Folder,
    SquareTerminal,
    Code,
    AppWindow,
    WinMinimize,
    WinMaximize,
    WinRestore,
    WinClose,
    ChevronRight,
    ChevronDown,
    ChevronUp,
    ChevronLeft,
    Ellipsis,
    Route,
    Lightbulb,
    Clock,
    DollarSign,
    CircleCheck,
    Bell,
    BellOff,
    Power,
    Download,
    FileText,
    Sheet,
    Provider(ProviderKind),
}

impl Icon {
    pub fn path(self) -> &'static str {
        match self {
            Icon::Refresh => "icons/refresh-cw.svg",
            Icon::Settings => "icons/settings.svg",
            Icon::Close => "icons/x.svg",
            Icon::Star => "icons/star.svg",
            Icon::ArrowUp => "icons/arrow-up.svg",
            Icon::ArrowDown => "icons/arrow-down.svg",
            Icon::Plus => "icons/plus.svg",
            Icon::Info => "icons/info.svg",
            Icon::EyeOff => "icons/eye-off.svg",
            Icon::Eye => "icons/eye.svg",
            Icon::Trash => "icons/trash.svg",
            Icon::Check => "icons/check.svg",
            Icon::ExternalLink => "icons/external-link.svg",
            Icon::LogIn => "icons/log-in.svg",
            Icon::Chart => "icons/chart-area.svg",
            Icon::Copy => "icons/copy.svg",
            Icon::Rocket => "icons/rocket.svg",
            Icon::Pencil => "icons/pencil.svg",
            Icon::FolderPlus => "icons/folder-plus.svg",
            Icon::Warning => "icons/triangle-alert.svg",
            Icon::Terminal => "icons/terminal.svg",
            Icon::Folder => "icons/folder.svg",
            Icon::SquareTerminal => "icons/square-terminal.svg",
            Icon::Code => "icons/code-xml.svg",
            Icon::AppWindow => "icons/app-window.svg",
            Icon::WinMinimize => "icons/win-minimize.svg",
            Icon::WinMaximize => "icons/win-maximize.svg",
            Icon::WinRestore => "icons/win-restore.svg",
            Icon::WinClose => "icons/win-close.svg",
            Icon::ChevronRight => "icons/chevron-right.svg",
            Icon::ChevronDown => "icons/chevron-down.svg",
            Icon::ChevronUp => "icons/chevron-up.svg",
            Icon::ChevronLeft => "icons/chevron-left.svg",
            Icon::Ellipsis => "icons/ellipsis.svg",
            Icon::Route => "icons/route.svg",
            Icon::Lightbulb => "icons/lightbulb.svg",
            Icon::Clock => "icons/clock.svg",
            Icon::DollarSign => "icons/dollar-sign.svg",
            Icon::CircleCheck => "icons/circle-check.svg",
            Icon::Bell => "icons/bell.svg",
            Icon::BellOff => "icons/bell-off.svg",
            Icon::Power => "icons/power.svg",
            Icon::Download => "icons/download.svg",
            Icon::FileText => "icons/file-text.svg",
            Icon::Sheet => "icons/sheet.svg",
            Icon::Provider(ProviderKind::Claude) => "icons/provider-claude.svg",
            Icon::Provider(ProviderKind::Codex) => "icons/provider-openai.svg",
            Icon::Provider(ProviderKind::Copilot) => "icons/provider-copilot.svg",
            Icon::Provider(ProviderKind::Cursor) => "icons/provider-cursor.svg",
            Icon::Provider(ProviderKind::Antigravity) => "icons/provider-antigravity.svg",
        }
    }
}

/// An icon `size` px square in `color`.
pub fn icon(i: Icon, size: f32, color: Hsla) -> Svg {
    svg()
        .path(i.path())
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// Series colour of a provider in the statistics and the limits board (owner-approved, checked
/// with the dataviz palette validator, 2026-11-07). A coloured dot or mark carries the identity;
/// text never takes it.
pub fn series_color(p: ProviderKind, dark: bool) -> Hsla {
    let hex = match (p, dark) {
        (ProviderKind::Claude, false) => 0xc96a4a,
        (ProviderKind::Codex, false) => 0x3f74cc,
        (ProviderKind::Claude, true) => 0xd07052,
        (ProviderKind::Codex, true) => 0x4f84dc,
        (ProviderKind::Copilot, false) => 0x8a63d2,
        (ProviderKind::Copilot, true) => 0x9d7ce3,
        (ProviderKind::Cursor, false) => 0x2a9686,
        (ProviderKind::Cursor, true) => 0x3cb0a0,
        (ProviderKind::Antigravity, false) => 0xc08a14,
        (ProviderKind::Antigravity, true) => 0xd6a23a,
    };
    gpui::rgb(hex).into()
}

/// Provider brand colour (Claude orange; Codex follows the text colour).
pub fn provider_color(p: ProviderKind, neutral: Hsla) -> Hsla {
    match p {
        ProviderKind::Claude => gpui::rgb(0xd97757).into(),
        ProviderKind::Codex | ProviderKind::Cursor => neutral,
        // Copilot's purple, Google blue.
        ProviderKind::Copilot => gpui::rgb(0x8957e5).into(),
        ProviderKind::Antigravity => gpui::rgb(0x4285f4).into(),
    }
}
