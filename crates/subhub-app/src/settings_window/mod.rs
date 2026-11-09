//! Settings window: category menu on the left, one content column on the right (owner rule).
//! Six fixed menu items (owner, 2026-11-07, "Settings distilled"): General, Appearance,
//! Accounts, Hub, Projects, About & updates. Accounts is a list; a row drills into that
//! account's page and "+ Add accounts" opens detection / import with a manual folder form.
use crate::format::Clip as _;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::Focusable as _;
use gpui::{
    AnyElement, Context, Entity, FontWeight, Hsla, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, Window, div, prelude::*, px,
};
use gpui_component::Disableable as _;
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::switch::Switch;
use subhub_core::detect::{DetectStatus, DetectedProfile};
use subhub_core::hover::{self, Health};
use subhub_core::model::{Account, ProviderKind};
use subhub_core::secrets;
use subhub_core::settings::{
    DisplaySettings, TaskbarAccounts, TaskbarMode, TaskbarMonitors, TaskbarPosition, ThemeMode,
    UsageStyle,
};
use uuid::Uuid;

use crate::icons::{Icon, icon, provider_color};
use crate::state::{AppState, HubStatus, SettingsTarget, provider_name};
use crate::taskbar::account_tag;
use crate::theme::{self, Tokens};

mod about;
mod accounts;
mod add_accounts;
mod appearance;
mod controls;
mod general;
mod hub;
mod projects;
use accounts::*;
use controls::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    General,
    Appearance,
    /// The account list.
    Accounts,
    /// One account's page, drilled into from the list (menu: Accounts).
    Account(Uuid),
    /// Detection / import plus the manual folder form (menu: Accounts).
    AddAccounts,
    Hub,
    Projects,
    About,
}

impl Page {
    /// The menu item that is highlighted for this page.
    fn menu(self) -> Page {
        match self {
            Page::Account(_) | Page::AddAccounts => Page::Accounts,
            p => p,
        }
    }
}

/// Tabs on Add accounts (owner, 2026-11-07).
#[derive(Clone, Copy, PartialEq, Eq)]
enum DetectTab {
    Cli,
    Hub,
}

/// The whole menu: fixed, no scrolling (owner, 2026-11-07).
const MENU: [(Page, &str); 6] = [
    (Page::General, "General"),
    (Page::Appearance, "Appearance"),
    (Page::Accounts, "Accounts"),
    (Page::Hub, "Hub"),
    (Page::Projects, "Projects"),
    (Page::About, "About & updates"),
];

const ACCENTS: [&str; 6] = [
    "#3b82f6", "#8b5cf6", "#14b8a6", "#22c55e", "#f97316", "#ec4899",
];

type Upd = Rc<dyn Fn(Box<dyn FnOnce(&mut DisplaySettings)>, &mut gpui::App)>;

pub struct SettingsWindow {
    state: Entity<AppState>,
    /// Opens the Statistics window (on `AppState::pending_overview_row`).
    on_stats: Rc<dyn Fn(&mut gpui::App)>,
    page: Page,
    /// Account page whose remove button is waiting for the second click.
    confirm_remove: Option<Uuid>,
    /// Hub page: the account whose "Remove from hub" waits for the second click.
    confirm_hub_remove: Option<Uuid>,
    /// Values the name/tag inputs were last filled with (to save only real edits).
    pop_name: String,
    pop_tag: String,
    rename: Entity<InputState>,
    group: Entity<InputState>,
    hub_url: Entity<InputState>,
    hub_key: Entity<InputState>,
    add_label: Entity<InputState>,
    add_dir: Entity<InputState>,
    tag: Entity<InputState>,
    add_provider: ProviderKind,
    // General → Advanced: CLI path inputs and the text they were last saved with.
    claude_path: Entity<InputState>,
    codex_path: Entity<InputState>,
    pop_claude: String,
    pop_codex: String,
    // Billing date on the account page, its saved text and the Renewal / Access ends choice.
    bill_date: Entity<InputState>,
    pop_bill: String,
    bill_renew: bool,
    // Projects page: the row whose label is being edited (one shared input), and the row whose
    // remove button is waiting for the second click.
    project_label: Entity<InputState>,
    editing_project: Option<Uuid>,
    confirm_remove_project: Option<Uuid>,
    message: Option<String>,
    /// Last seen `AppState::user_saves`, and until when the footer says "Settings saved".
    seen_saves: u64,
    saved_until: Option<std::time::Instant>,
    /// The message the toast last saw, how long it shows, and a dismissed problem text.
    toast_message: Option<String>,
    message_until: Option<std::time::Instant>,
    dismissed: Option<String>,
    /// Add accounts page: the last detection and the ticked folders.
    detected: Vec<DetectedProfile>,
    picked: HashSet<PathBuf>,
    /// The account list the detection was computed against (`accounts_sig`).
    detect_sig: u64,
    detect_tab: DetectTab,
    /// Add accounts: the manual folder form is shown ("Choose a folder manually…").
    manual_add: bool,
    /// General → Advanced (sign-in terminal, CLI paths); closed by default.
    advanced_open: bool,
    /// Appearance → More options (position, monitors, hover cards); closed by default.
    more_open: bool,
    _subs: Vec<Subscription>,
}

impl SettingsWindow {
    pub fn new(
        state: Entity<AppState>,
        on_stats: Rc<dyn Fn(&mut gpui::App)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_saves = state.read(cx).user_saves;
        let url = state.read(cx).settings.hub.base_url.clone();
        let path_text = |p: Option<&std::path::PathBuf>| {
            p.map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let (claude_text, codex_text) = {
            let tools = &state.read(cx).settings.tools;
            (
                path_text(tools.claude_exe.as_ref()),
                path_text(tools.codex_exe.as_ref()),
            )
        };
        let claude_path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Auto-detect")
                .default_value(claude_text.clone())
        });
        let codex_path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Auto-detect")
                .default_value(codex_text.clone())
        });
        let bill_date = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD"));
        let project_label = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));
        // "Manage projects…" from a Launch menu opens this window straight on Projects.
        let start_page = match state.update(cx, |s, _| s.pending_settings_page.take()) {
            Some(target) => target_page(target),
            None => Page::General,
        };
        let rename = cx.new(|cx| InputState::new(window, cx).placeholder("Account name"));
        let group = cx.new(|cx| InputState::new(window, cx).placeholder("New group, e.g. Work"));
        let tag = cx.new(|cx| InputState::new(window, cx).placeholder("Auto"));
        let hub_url = cx.new(|cx| InputState::new(window, cx).default_value(url));
        let hub_key = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Paste the management key")
        });
        let add_label = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        let add_dir = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Profile folder, e.g. C:\\Users\\you\\.claude-work")
        });
        let saves =
            |ev: &InputEvent| matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur);
        let subs = vec![
            cx.observe_in(&state, window, |this, state, window, cx| {
                // Messages produced inside the state (e.g. "Linked the profile to …") go to the toast.
                if let Some(m) = state.update(cx, |s, _| s.message_for_user.take()) {
                    this.message = Some(m);
                }
                // A Launch menu asked for a page while this window was already open.
                if let Some(target) = state.update(cx, |s, _| s.pending_settings_page.take()) {
                    this.go(target_page(target), window, cx);
                }
                this.redetect_if_accounts_changed(cx);
                // A change the user made was saved: show the "Saved" toast.
                let saves = state.read(cx).user_saves;
                if saves != this.seen_saves {
                    this.seen_saves = saves;
                    let until = std::time::Instant::now() + std::time::Duration::from_millis(2500);
                    this.saved_until = Some(until);
                    cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(2600))
                            .await;
                        let _ = this.update(cx, |_, cx| cx.notify());
                    })
                    .detach();
                }
                cx.notify();
            }),
            cx.subscribe(&rename, move |this, _, ev: &InputEvent, cx| {
                if saves(ev) {
                    this.commit(cx);
                }
            }),
            cx.subscribe(&tag, move |this, _, ev: &InputEvent, cx| {
                if saves(ev) {
                    this.commit(cx);
                }
            }),
            cx.subscribe_in(
                &group,
                window,
                move |this, input, ev: &InputEvent, window, cx| {
                    if saves(ev)
                        && let Page::Account(id) = this.page
                    {
                        let v = input.read(cx).value().trim().to_string();
                        if !v.is_empty() {
                            this.state.update(cx, |s, cx| {
                                s.set_group(id, Some(v));
                                cx.notify();
                            });
                            input.update(cx, |i, cx| i.set_value("", window, cx));
                        }
                    }
                },
            ),
            cx.subscribe(&bill_date, move |this, _, ev: &InputEvent, cx| {
                if saves(ev) {
                    this.commit(cx);
                }
            }),
            cx.subscribe(&project_label, move |this, _, ev: &InputEvent, cx| {
                if saves(ev) {
                    this.commit_project_label(cx);
                }
            }),
            cx.subscribe(&claude_path, move |this, _, ev: &InputEvent, cx| {
                if saves(ev) {
                    this.commit_cli_path(ProviderKind::Claude, cx);
                }
            }),
            cx.subscribe(&codex_path, move |this, _, ev: &InputEvent, cx| {
                if saves(ev) {
                    this.commit_cli_path(ProviderKind::Codex, cx);
                }
            }),
            cx.subscribe(&hub_url, move |this, input, ev: &InputEvent, cx| {
                if saves(ev) {
                    let v = input.read(cx).value().to_string();
                    this.state.update(cx, |s, cx| {
                        s.set_hub_url(&v);
                        cx.notify();
                    });
                }
            }),
        ];
        let mut this = Self {
            state,
            on_stats,
            page: start_page,
            detected: vec![],
            picked: HashSet::new(),
            detect_sig: 0,
            detect_tab: DetectTab::Cli,
            manual_add: false,
            advanced_open: false,
            more_open: false,
            confirm_remove: None,
            confirm_hub_remove: None,
            project_label,
            editing_project: None,
            confirm_remove_project: None,
            pop_name: String::new(),
            pop_tag: String::new(),
            rename,
            group,
            tag,
            hub_url,
            hub_key,
            add_label,
            add_dir,
            add_provider: ProviderKind::Claude,
            claude_path,
            codex_path,
            pop_claude: claude_text,
            pop_codex: codex_text,
            bill_date,
            pop_bill: String::new(),
            bill_renew: true,
            message: None,
            seen_saves: initial_saves,
            saved_until: None,
            toast_message: None,
            message_until: None,
            dismissed: None,
            _subs: subs,
        };
        match this.page {
            Page::AddAccounts => this.open_add_accounts(cx),
            // Opened from a card's ⋯ menu: fill the account page's fields as `go` does.
            Page::Account(id) => {
                this.page = Page::General;
                this.go(Page::Account(id), window, cx);
            }
            _ => {}
        }
        this
    }

    /// Entering Add accounts: detect again; the manual form starts open only when detection
    /// found nothing.
    fn open_add_accounts(&mut self, cx: &mut Context<Self>) {
        self.run_detect(cx);
        self.manual_add = self.detected.is_empty();
    }

    /// Saves edited name/tag of the open account page (Enter, blur, page switch).
    fn commit(&mut self, cx: &mut Context<Self>) {
        let Page::Account(id) = self.page else {
            return;
        };
        let name = self.rename.read(cx).value().to_string();
        let tag = self.tag.read(cx).value().to_string();
        let bill = self.bill_date.read(cx).value().to_string();
        if bill != self.pop_bill {
            self.commit_billing_date(id, bill, cx);
        }
        let name_changed = name != self.pop_name;
        let tag_changed = tag != self.pop_tag;
        if !name_changed && !tag_changed {
            return;
        }
        self.pop_name = name.clone();
        self.pop_tag = tag.clone();
        self.state.update(cx, |s, cx| {
            if name_changed {
                s.rename(id, &name);
            }
            if tag_changed {
                s.set_account_tag(id, &tag);
            }
            cx.notify();
        });
    }

    /// Saves the billing date typed on the account page. Empty is ignored (Clear removes it).
    fn commit_billing_date(&mut self, id: Uuid, text: String, cx: &mut Context<Self>) {
        self.pop_bill = text.clone();
        if text.trim().is_empty() {
            return;
        }
        let Some(date) = subhub_core::billing::parse_date(&text) else {
            self.message = Some("Use the date as YYYY-MM-DD, e.g. 2026-12-01.".into());
            cx.notify();
            return;
        };
        let renew = self.bill_renew;
        let r = self.state.update(cx, |s, cx| {
            let r = s.set_billing(id, date, renew);
            if r.is_ok() {
                s.check_billing_reminders(cx);
            }
            cx.notify();
            r
        });
        if let Err(e) = r {
            self.message = Some(e);
        }
        cx.notify();
    }

    /// Re-fills the billing input from the saved state (shows the next, possibly estimated, date).
    fn fill_billing(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let (text, renew) = {
            let s = self.state.read(cx);
            match s.settings.billing.get(&id) {
                Some(b) => (
                    subhub_core::billing::next_date(b, AppState::today())
                        .format("%Y-%m-%d")
                        .to_string(),
                    b.auto_renew,
                ),
                None => (String::new(), true),
            }
        };
        self.pop_bill = text.clone();
        self.bill_renew = renew;
        self.bill_date
            .update(cx, |i, cx| i.set_value(text, window, cx));
    }

    /// Saves a CLI path typed in General → Advanced (Enter, blur) when it changed.
    fn commit_cli_path(&mut self, provider: ProviderKind, cx: &mut Context<Self>) {
        let (input, last) = match provider {
            ProviderKind::Claude => (&self.claude_path, &mut self.pop_claude),
            ProviderKind::Codex => (&self.codex_path, &mut self.pop_codex),
            _ => return,
        };
        let text = input.read(cx).value().to_string();
        if text == *last {
            return;
        }
        *last = text.clone();
        let r = self.state.update(cx, |s, cx| {
            let r = s.set_cli_path(provider, &text);
            cx.notify();
            r
        });
        if let Err(e) = r {
            self.message = Some(e);
        }
        cx.notify();
    }

    /// Saves the project label being edited (Enter, blur, page switch) and ends editing.
    fn commit_project_label(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.editing_project.take() else {
            return;
        };
        let text = self.project_label.read(cx).value().to_string();
        self.state.update(cx, |s, cx| {
            s.rename_project(id, &text);
            cx.notify();
        });
        cx.notify();
    }

    /// Folder picker; every chosen folder becomes a project unless it is already saved.
    fn pick_project_folders(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add project".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let (mut added, mut problem) = (0, None);
                for dir in paths {
                    let r = this.state.update(cx, |s, cx| {
                        let r = s.add_project(dir);
                        cx.notify();
                        r
                    });
                    match r {
                        Ok(()) => added += 1,
                        Err(e) => problem = Some(e),
                    }
                }
                if let Some(e) = problem {
                    this.message = Some(e);
                } else if added > 1 {
                    this.message = Some(format!("Added {added} projects."));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn go(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == page {
            return;
        }
        self.commit(cx);
        self.commit_project_label(cx);
        self.page = page;
        self.confirm_remove = None;
        self.confirm_hub_remove = None;
        self.confirm_remove_project = None;
        self.message = None;
        if page == Page::AddAccounts {
            self.open_add_accounts(cx);
        }
        if let Page::Account(id) = page {
            let (label, own) = {
                let s = self.state.read(cx);
                (
                    s.account(id).map(|a| a.label.clone()).unwrap_or_default(),
                    s.settings
                        .account_tags
                        .get(&id)
                        .cloned()
                        .unwrap_or_default(),
                )
            };
            self.pop_name = label.clone();
            self.pop_tag = own.clone();
            self.rename
                .update(cx, |i, cx| i.set_value(label, window, cx));
            self.tag.update(cx, |i, cx| i.set_value(own, window, cx));
            self.group.update(cx, |i, cx| i.set_value("", window, cx));
            self.fill_billing(id, window, cx);
        }
        cx.notify();
    }

    fn updater(&self) -> Upd {
        let st = self.state.clone();
        Rc::new(move |f, cx| {
            st.update(cx, |s, cx| {
                s.update_display(f);
                cx.notify();
            });
            cx.refresh_windows();
        })
    }
}

/// The page a request from another window ("Manage projects…", first start) opens.
fn target_page(target: SettingsTarget) -> Page {
    match target {
        SettingsTarget::General => Page::General,
        SettingsTarget::Appearance => Page::Appearance,
        SettingsTarget::Accounts => Page::Accounts,
        SettingsTarget::Hub => Page::Hub,
        SettingsTarget::About => Page::About,
        SettingsTarget::Projects => Page::Projects,
        SettingsTarget::AddAccounts => Page::AddAccounts,
        SettingsTarget::Account(id) => Page::Account(id),
    }
}

impl SettingsWindow {
    /// The label of the menu item for the page shown (the main window's sidebar highlight).
    pub(crate) fn active_menu(&self) -> &'static str {
        let page = self.page.menu();
        MENU.iter()
            .find(|(p, _)| *p == page)
            .map_or("General", |(_, l)| *l)
    }
}

/// The settings pages for the main window's sidebar: label and where it goes.
pub(crate) const MENU_TARGETS: [(&str, SettingsTarget); 6] = [
    ("General", SettingsTarget::General),
    ("Appearance", SettingsTarget::Appearance),
    ("Accounts", SettingsTarget::Accounts),
    ("Hub", SettingsTarget::Hub),
    ("Projects", SettingsTarget::Projects),
    ("About & updates", SettingsTarget::About),
];

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let d = self.state.read(cx).display().clone();
        let dark = theme::is_dark(d.theme, cx);
        let t = Tokens::new(dark, &d.accent);
        // An open account page whose account vanished falls back to the account list.
        if let Page::Account(id) = self.page
            && self.state.read(cx).account(id).is_none()
        {
            self.page = Page::Accounts;
        }

        let content: AnyElement = match self.page {
            Page::General => self.general(&t, cx).into_any_element(),
            Page::Appearance => self.appearance_page(&t, cx).into_any_element(),
            Page::Accounts => self.accounts_page(&t, cx).into_any_element(),
            Page::Account(id) => self.account_page(id, &t, cx),
            Page::AddAccounts => self.add_accounts_page(&t, cx).into_any_element(),
            Page::Hub => self.hub(&t, cx).into_any_element(),
            Page::Projects => self.projects_page(&t, cx).into_any_element(),
            Page::About => self.about_page(&t, cx).into_any_element(),
        };
        let toast = self.toast(&t, cx);

        // Only the page: the main window draws the title bar and the sidebar (owner, 2026-11-08:
        // Statistics and Settings are one window). Content scrolls; a toast floats over its
        // bottom edge.
        let _ = window;
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.text)
            .child(
                div()
                    .id("content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_6()
                            // Same column as the Insights pages (owner, 2026-11-08: Settings
                            // kept the old window's 680 px): 1000 px board plus gutter, centred,
                            // with the Overview's insets (20 + 28 left, 32 + 28 right).
                            .w_full()
                            .max_w(px(1012.0))
                            .mx_auto()
                            .pl(px(48.0))
                            .pr(px(60.0))
                            .pt_8()
                            .pb(px(72.0))
                            .child(content),
                    ),
            )
            .when_some(toast, |d, toast| d.child(toast))
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ToastKind {
    Success,
    Info,
    Error,
}

impl SettingsWindow {
    /// Floating toast: a small raised card with an icon by kind and a
    /// short title. Changes save quietly; "Saved" shows for ~2 s after a change, an action's
    /// message for ~4 s, problems (settings file, hub steering) until dismissed with ✕.
    fn toast(&mut self, t: &Tokens, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let now = std::time::Instant::now();
        // A new action message starts its own timer.
        if self.message != self.toast_message {
            self.toast_message = self.message.clone();
            if let Some(m) = &self.message {
                // ~4 s, longer for long messages (about 15 characters a second to read).
                let ms = (4000 + m.chars().count().saturating_sub(60) as u64 * 65).min(10_000);
                self.message_until = Some(now + std::time::Duration::from_millis(ms));
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(ms + 100))
                        .await;
                    let _ = this.update(cx, |_, cx| cx.notify());
                })
                .detach();
            }
        }
        let s = self.state.read(cx);
        let problem = s
            .notice
            .clone()
            .or(s
                .steering_error
                .clone()
                .map(|e| format!("Hub steering: {e}")))
            .filter(|p| self.dismissed.as_ref() != Some(p));
        let (kind, text) = if let Some(p) = problem {
            (ToastKind::Error, p)
        } else if let Some(m) = self
            .message
            .clone()
            .filter(|_| self.message_until.is_some_and(|u| now < u))
        {
            (ToastKind::Info, m)
        } else if self.saved_until.is_some_and(|u| now < u) {
            (ToastKind::Success, "Saved".to_string())
        } else {
            return None;
        };
        let (icon_name, icon_color) = match kind {
            ToastKind::Success => (Icon::Check, t.ok),
            ToastKind::Info => (Icon::Info, t.accent),
            ToastKind::Error => (Icon::Close, t.danger),
        };
        let raised: Hsla = if t.dark {
            gpui::rgb(0x2c2c32).into()
        } else {
            gpui::white()
        };
        let close = (kind == ToastKind::Error).then(|| {
            let text = text.clone();
            div()
                .id("toast-close")
                .size(px(22.0))
                .rounded(px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|d| d.bg(t.surface_hover))
                .child(icon(Icon::Close, 13.0, t.muted))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dismissed = Some(text.clone());
                    cx.notify();
                }))
        });
        Some(
            div()
                .absolute()
                .bottom(px(18.0))
                .left_0()
                .right_0()
                .px(px(16.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .max_w(px(520.0))
                        .pl(px(10.0))
                        .pr(px(if close.is_some() { 6.0 } else { 14.0 }))
                        .py(px(7.0))
                        .rounded(px(10.0))
                        .bg(raised)
                        .border_1()
                        .border_color(t.border)
                        .shadow_lg()
                        .child(
                            div()
                                .flex_none()
                                .size(px(20.0))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(icon_color.opacity(0.16))
                                .child(icon(icon_name, 13.0, icon_color)),
                        )
                        // Shrinks and wraps inside the card instead of running past its edge.
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .child(SharedString::from(text)),
                        )
                        .when_some(close, |d, c| d.child(c.flex_none())),
                )
                .into_any_element(),
        )
    }
}
