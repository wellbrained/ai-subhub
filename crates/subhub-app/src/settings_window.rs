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
                            .p_8()
                            .pb(px(72.0))
                            .max_w(px(680.0))
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
            if self.message.is_some() {
                self.message_until = Some(now + std::time::Duration::from_secs(4));
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(4100))
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
                .flex()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .max_w(px(460.0))
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
                                .size(px(20.0))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(icon_color.opacity(0.16))
                                .child(icon(icon_name, 13.0, icon_color)),
                        )
                        .child(div().text_sm().child(SharedString::from(text)))
                        .when_some(close, |d, c| d.child(c)),
                )
                .into_any_element(),
        )
    }
}

// ---------- building blocks ----------

fn heading(text: &str) -> impl IntoElement {
    div()
        .pb_2()
        .text_lg()
        .font_weight(FontWeight::SEMIBOLD)
        .child(SharedString::from(text.to_string()))
}

fn row(label: &str, hint: Option<&str>, control: impl IntoElement, t: &Tokens) -> impl IntoElement {
    div()
        .flex()
        // Wrap: when label + control don't fit side by side, the control moves onto its own
        // line, right-aligned, instead of spilling past the edge (chips clipped "25%").
        .flex_wrap()
        .items_center()
        .gap_x_4()
        .gap_y_2()
        .py_3()
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .flex_1()
                .min_w(px(240.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(div().text_sm().child(SharedString::from(label.to_string())))
                .when_some(hint.map(|h| h.to_string()), |d, h| {
                    d.child(div().text_xs().text_color(t.muted).child(h))
                }),
        )
        .child(div().flex_none().ml_auto().child(control))
}

/// Like `row`, but the control sits under the label (wide controls: chips, lists).
fn block(
    label: &str,
    hint: Option<&str>,
    content: impl IntoElement,
    t: &Tokens,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .py_3()
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(div().text_sm().child(SharedString::from(label.to_string())))
                .when_some(hint.map(|h| h.to_string()), |d, h| {
                    d.child(div().text_xs().text_color(t.muted).child(h))
                }),
        )
        .child(content)
}

/// Segmented chips for an enum-like choice.
fn chips<T: PartialEq + Clone + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    current: &T,
    t: &Tokens,
    on_pick: impl Fn(T, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let on_pick = Rc::new(on_pick);
    let mut bar = div()
        .flex()
        .flex_none()
        .gap_1()
        .p(px(3.0))
        .rounded(px(10.0))
        .bg(t.surface);
    for (i, (value, label)) in options.iter().enumerate() {
        let active = value == current;
        let value = value.clone();
        let on_pick = on_pick.clone();
        bar = bar.child(
            div()
                .id((id, i))
                .px_3()
                .py(px(4.0))
                .rounded(px(7.0))
                .text_xs()
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(t.bg)
                        .text_color(t.text)
                        .font_weight(FontWeight::MEDIUM)
                        .shadow_sm()
                })
                .when(!active, |d| {
                    d.text_color(t.muted).hover(|s| s.text_color(t.text))
                })
                .child(*label)
                .on_click(move |_, _, cx| on_pick(value.clone(), cx)),
        );
    }
    bar
}

fn button(
    id: impl Into<gpui::ElementId>,
    label: &str,
    t: &Tokens,
    primary: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_3()
        .py(px(5.0))
        .rounded(px(8.0))
        .text_xs()
        .cursor_pointer()
        .when(primary, |d| {
            d.bg(t.accent)
                .text_color(gpui::white())
                .hover(|s| s.opacity(0.9))
        })
        .when(!primary, |d| {
            d.bg(t.surface)
                .text_color(t.text)
                .hover(|s| s.bg(t.surface_hover))
        })
        .child(SharedString::from(label.to_string()))
}

/// A secondary button with a drawn icon before the label.
fn icon_button(
    id: impl Into<gpui::ElementId>,
    ic: Icon,
    label: &str,
    t: &Tokens,
) -> gpui::Stateful<gpui::Div> {
    button(id, label, t, false).child(icon(ic, 12.0, t.muted))
}

/// A small square icon-only button; `enabled` false dims it and ignores clicks' visual hover.
fn square_button(
    id: impl Into<gpui::ElementId>,
    ic: Icon,
    tip: &'static str,
    enabled: bool,
    t: &Tokens,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(26.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .bg(t.surface)
        .when(enabled, |d| {
            d.cursor_pointer().hover(|s| s.bg(t.surface_hover))
        })
        .when(!enabled, |d| d.opacity(0.35))
        .child(icon(ic, 13.0, t.muted))
        .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tip).build(window, cx))
}

fn tag(text: &str, color: Hsla, t: &Tokens) -> impl IntoElement {
    div()
        .px(px(6.0))
        .py(px(1.0))
        .rounded(px(6.0))
        .bg(t.surface)
        .text_xs()
        .text_color(color)
        .child(SharedString::from(text.to_string()))
}

/// The account's taskbar tag as a coloured chip.
fn tag_chip(text: &str, color: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .min_w(px(26.0))
        .px(px(5.0))
        .py(px(1.0))
        .rounded(px(6.0))
        .bg(color)
        .text_color(gpui::white())
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .flex()
        .justify_center()
        .child(SharedString::from(text.to_string()))
}

/// A section heading inside a page ("Popover", "Next to the tray").
fn section(text: &str, t: &Tokens) -> impl IntoElement {
    div()
        .pt_5()
        .pb_1()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.faint)
        .child(SharedString::from(text.to_uppercase()))
}

/// A collapsible group header ("Advanced", "More options"): a chevron that points right while
/// closed and down while open, the label, and a muted summary of what is inside while closed.
/// The caller adds the click.
fn disclosure(
    id: &'static str,
    label: &str,
    summary: &str,
    open: bool,
    t: &Tokens,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .mt_2()
        .py_2()
        .px_2()
        .mx(px(-8.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(t.surface))
        .active(|s| s.bg(t.surface_hover))
        .child(icon(
            if open {
                Icon::ChevronDown
            } else {
                Icon::ChevronRight
            },
            14.0,
            t.muted,
        ))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(SharedString::from(label.to_string())),
        )
        .when(!open, |d| {
            d.child(
                div()
                    .min_w_0()
                    .text_xs()
                    .text_color(t.muted)
                    .clip("summary", SharedString::from(summary.to_string())),
            )
        })
}

/// "‹ Accounts" at the top of a page drilled into from the account list. The caller adds the
/// click.
fn back_link(id: &'static str, label: &str, t: &Tokens) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .mb_3()
        .ml(px(-6.0))
        .pl(px(2.0))
        .pr_2()
        .py_1()
        .rounded(px(6.0))
        .text_sm()
        .text_color(t.muted)
        .cursor_pointer()
        .hover(|s| s.bg(t.surface).text_color(t.text))
        .active(|s| s.bg(t.surface_hover))
        .child(icon(Icon::ChevronLeft, 14.0, t.muted))
        .child(SharedString::from(label.to_string()))
}

// ---------- pages ----------

impl SettingsWindow {
    fn general(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let minutes = self.state.read(cx).settings.refresh_minutes;
        let state = self.state.clone();
        div()
            .flex()
            .flex_col()
            .child(heading("General"))
            .child(row(
                "Refresh every",
                Some("Failures back off automatically; provider cooldowns are always respected."),
                chips("refresh", &[(1u32, "1m"), (2, "2m"), (5, "5m"), (10, "10m"), (15, "15m"), (30, "30m")], &minutes, t, move |m, cx| {
                    state.update(cx, |s, cx| {
                        s.set_refresh_minutes(m);
                        cx.notify();
                    })
                }),
                t,
            ))
            .child({
                // Warm-up is set up next to its explanation (Statistics → 5-hour windows); here
                // only its state and the way there.
                let s = self.state.read(cx);
                let w = &s.settings.warmup;
                let hint = if w.enabled {
                    let names: Vec<String> = w
                        .accounts
                        .iter()
                        .filter_map(|id| s.account(*id))
                        .map(|a| {
                            // Same name on both providers: say which.
                            let twin = s.settings.accounts.iter().any(|b| {
                                b.id != a.id && b.label.trim().eq_ignore_ascii_case(a.label.trim())
                            });
                            if twin {
                                format!("{} ({})", a.label, crate::state::provider_name(a.provider))
                            } else {
                                a.label.clone()
                            }
                        })
                        .collect();
                    format!(
                        "On · {} {}{}{}",
                        w.time.format("%-H:%M"),
                        w.days_text(),
                        if names.is_empty() {
                            " · no account chosen".to_string()
                        } else {
                            format!(" · {}", names.join(", "))
                        },
                        if w.wake_pc { " · wakes the PC" } else { "" }
                    )
                } else {
                    "Off. Starts your 5-hour windows early, so they reset when you usually run out."
                        .to_string()
                };
                let state = self.state.clone();
                let on_stats = self.on_stats.clone();
                row(
                    "Warm-up",
                    Some(&hint),
                    button("warmup-setup", "Open 5-hour windows", t, false).on_click(
                        move |_, _, cx| {
                            state.update(cx, |s, _| {
                                s.pending_overview_row =
                                    Some(subhub_core::insights::TipRow::Windows)
                            });
                            on_stats(cx);
                        },
                    ),
                    t,
                )
                .into_any_element()
            })
            .child({
                let s = self.state.read(cx);
                let steering = s.settings.steering.enabled
                    && s.settings.accounts.iter().any(|a| a.hub.is_some());
                let hint = if steering {
                    "A notification about 15 minutes before an account's 5-hour window runs out, when another account of the same provider has a fresh one. At most one per window. Off for hub accounts while hub steering is on: the hub moves on by itself."
                } else {
                    "A notification about 15 minutes before an account's 5-hour window runs out, when another account of the same provider has a fresh one. At most one per window."
                };
                let state = self.state.clone();
                row(
                    "Tell me before a window runs out",
                    Some(hint),
                    Switch::new("nudge")
                        .checked(s.settings.nudge.enabled)
                        .on_click(move |v, _, cx| {
                            let v = *v;
                            state.update(cx, |s, cx| {
                                s.set_nudge(v);
                                cx.notify();
                            });
                        }),
                    t,
                )
                .into_any_element()
            })
            .child(row(
                "Close button",
                Some("What ✕ on the popover and this window's close button do. Quit is always in the tray icon's menu."),
                chips(
                    "closeq",
                    &[(false, "Keep running in tray"), (true, "Quit completely")],
                    &self.state.read(cx).display().close_quits,
                    t,
                    {
                        let st = self.state.clone();
                        move |v, cx| {
                            st.update(cx, |s, cx| {
                                s.update_display(|d| d.close_quits = v);
                                cx.notify();
                            })
                        }
                    },
                ),
                t,
            ))
            .child(row(
                "Start with Windows",
                Some(if crate::demo::on() {
                    crate::demo::NOT_AVAILABLE
                } else {
                    "Opens SubHub in the tray when you sign in to Windows. Per user, no administrator rights; the switch shows what Windows has registered."
                }),
                Switch::new("startup")
                    .checked(crate::startup::is_enabled())
                    .disabled(crate::demo::on())
                    .on_click({
                        let me = cx.entity();
                        move |v, _, cx| {
                            let v = *v;
                            me.update(cx, |this, cx| {
                                match crate::startup::set_enabled(v) {
                                    Ok(()) => this.state.update(cx, |s, cx| {
                                        s.user_changed();
                                        cx.notify();
                                    }),
                                    Err(e) => {
                                        this.message =
                                            Some(format!("Could not change startup: {e}"));
                                    }
                                }
                                cx.notify();
                            })
                        }
                    }),
                t,
            ))
            .child(
                disclosure(
                    "advanced",
                    "Advanced",
                    "Sign-in terminal, Claude and Codex CLI paths",
                    self.advanced_open,
                    t,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.advanced_open = !this.advanced_open;
                    cx.notify();
                })),
            )
            .when(self.advanced_open, |d| d.child(self.advanced(t, cx)))
    }

    /// Appearance = the popover's look plus the display next to the tray (owner, 2026-11-07).
    /// Rarely changed taskbar rows sit behind "More options".
    fn appearance_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let d = self.state.read(cx).display().clone();
        let u = self.updater();
        let mut accent_row = div().flex().gap_2();
        for (i, hex) in ACCENTS.iter().enumerate() {
            let active = d.accent.eq_ignore_ascii_case(hex);
            let u = u.clone();
            let color = theme::parse_hex(hex).unwrap_or(t.accent);
            accent_row = accent_row.child(
                div()
                    .id(("accent", i))
                    .size(px(20.0))
                    .rounded_full()
                    .bg(color)
                    .cursor_pointer()
                    .border_2()
                    .border_color(if active { t.text } else { t.bg })
                    .on_click(move |_, _, cx| u(Box::new(move |d| d.accent = hex.to_string()), cx)),
            );
        }
        // One setter per control (each closure owns its handle).
        let set = |f: fn(&mut DisplaySettings, bool)| {
            let u = u.clone();
            move |v: &bool, _: &mut Window, cx: &mut gpui::App| {
                let v = *v;
                u(Box::new(move |d| f(d, v)), cx)
            }
        };

        let popover = div()
            .flex()
            .flex_col()
            .child(section("Popover", t))
            .child(row(
                "Theme",
                None,
                chips(
                    "theme",
                    &[
                        (ThemeMode::System, "System"),
                        (ThemeMode::Light, "Light"),
                        (ThemeMode::Dark, "Dark"),
                    ],
                    &d.theme,
                    t,
                    {
                        let u = u.clone();
                        move |v, cx| {
                            u(Box::new(move |d| d.theme = v), cx);
                            let dark = theme::is_dark(v, cx);
                            theme::apply_component_theme(dark, None, cx);
                        }
                    },
                ),
                t,
            ))
            .child(row("Accent color", None, accent_row, t))
            .child(row(
                "Show usage as",
                Some("Applies to the popover, the taskbar and the hover cards."),
                chips(
                    "style",
                    &[
                        (UsageStyle::Remaining, "% left"),
                        (UsageStyle::Used, "% used"),
                    ],
                    &d.usage_style,
                    t,
                    {
                        let u = u.clone();
                        move |v, cx| u(Box::new(move |d| d.usage_style = v), cx)
                    },
                ),
                t,
            ))
            .child(row(
                "Plan badges",
                None,
                Switch::new("plan")
                    .checked(d.show_plan)
                    .on_click(set(|d, v| d.show_plan = v)),
                t,
            ))
            .child(row(
                "Show emails",
                Some("Under the account names in the popover."),
                Switch::new("email")
                    .checked(d.show_email)
                    .on_click(set(|d, v| d.show_email = v)),
                t,
            ))
            .child(row(
                "Close popover when clicking elsewhere",
                Some("Off: it stays open until you click the tray icon, press Esc or use the close button."),
                Switch::new("blur-close")
                    .checked(d.close_popover_on_blur)
                    .on_click(set(|d, v| d.close_popover_on_blur = v)),
                t,
            ));

        let mut tray = div()
            .flex()
            .flex_col()
            .child(section("Next to the tray", t))
            .child(row(
                "Show usage on the taskbar",
                Some("Always visible next to the tray (hidden during fullscreen); click to open the popover, drag to move. Off: just the tray icon."),
                Switch::new("tb-on")
                    .checked(d.taskbar_enabled)
                    .on_click(set(|d, v| d.taskbar_enabled = v)),
                t,
            ));
        // Style, accounts and the rest only matter while the display is on.
        if d.taskbar_enabled {
            tray = tray
                .child(row(
                    "Style",
                    Some("Badge: provider, account tag, 5-hour and weekly. Bars: name and the 5-hour bar."),
                    chips(
                        "tbm",
                        &[(TaskbarMode::Badge, "Badge"), (TaskbarMode::Bars, "Bars")],
                        &d.taskbar_mode,
                        t,
                        {
                            let u = u.clone();
                            move |v, cx| u(Box::new(move |d| d.taskbar_mode = v), cx)
                        },
                    ),
                    t,
                ))
                .child(self.taskbar_accounts_block(&d, t))
                .child(
                    disclosure(
                        "more-options",
                        "More options",
                        "Position, monitors, details on hover",
                        self.more_open,
                        t,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.more_open = !this.more_open;
                        cx.notify();
                    })),
                )
                .when(self.more_open, |page| {
                    page.child(row(
                        "Position",
                        Some("Left sits next to the weather/widgets button, right next to the tray icons. Dragging the display moves it too."),
                        chips(
                            "tbp",
                            &[
                                (TaskbarPosition::Left, "Left"),
                                (TaskbarPosition::Right, "Right"),
                            ],
                            &d.taskbar_position,
                            t,
                            {
                                let u = u.clone();
                                move |v, cx| {
                                    u(
                                        Box::new(move |d| {
                                            d.taskbar_position = v;
                                            d.reset_taskbar_offsets();
                                        }),
                                        cx,
                                    )
                                }
                            },
                        ),
                        t,
                    ))
                    .child(self.monitors_block(&d, t))
                    .child(row(
                        "Show details on hover",
                        Some("A small card over each taskbar badge: both windows, reset times and status."),
                        Switch::new("hover-on")
                            .checked(d.hover_details)
                            .on_click(set(|d, v| d.hover_details = v)),
                        t,
                    ))
                });
        }

        div()
            .flex()
            .flex_col()
            .child(heading("Appearance"))
            .child(popover)
            .child(tray)
            .child(
                div().pt_6().child(
                    button("reset-display", "Reset appearance", t, false)
                        .on_click(move |_, _, cx| u(Box::new(|d| *d = Default::default()), cx)),
                ),
            )
    }

    /// Which accounts the taskbar shows, compact: Favorites / Focus / Auto and, except for
    /// Focus (one account), how many.
    fn taskbar_accounts_block(&self, d: &DisplaySettings, t: &Tokens) -> impl IntoElement {
        let u = self.updater();
        let u2 = u.clone();
        let hint = match d.taskbar_accounts {
            TaskbarAccounts::Favorites => {
                "Favorites: the accounts starred in the popover (or switched on under Show on taskbar), in star order."
            }
            TaskbarAccounts::Focus => "Focus: only the account new hub threads go to.",
            TaskbarAccounts::Auto => {
                "Auto: the account new hub threads go to, plus any running low (25% left or less)."
            }
        };
        let controls = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(chips(
                "tba",
                &[
                    (TaskbarAccounts::Favorites, "Favorites"),
                    (TaskbarAccounts::Focus, "Focus"),
                    (TaskbarAccounts::Auto, "Auto"),
                ],
                &d.taskbar_accounts,
                t,
                move |v, cx| u(Box::new(move |d| d.taskbar_accounts = v), cx),
            ))
            .when(d.taskbar_accounts != TaskbarAccounts::Focus, |c| {
                c.child(div().pl_2().text_xs().text_color(t.muted).child("up to"))
                    .child(chips(
                        "tbc",
                        &[(1u8, "1"), (2, "2"), (3, "3"), (4, "4"), (5, "5")],
                        &d.taskbar_count,
                        t,
                        move |v, cx| u2(Box::new(move |d| d.taskbar_count = v), cx),
                    ))
            });
        block("Accounts", Some(hint), controls, t)
    }

    /// Appearance → More options → Monitors: main taskbar, all taskbars, or chosen monitors.
    fn monitors_block(&self, d: &DisplaySettings, t: &Tokens) -> impl IntoElement {
        #[derive(Clone, PartialEq)]
        enum Pick {
            Main,
            All,
            Choose,
        }
        let monitors = crate::monitors::list();
        let current = match &d.taskbar_monitors {
            TaskbarMonitors::Main => Pick::Main,
            TaskbarMonitors::All => Pick::All,
            TaskbarMonitors::Chosen(_) => Pick::Choose,
        };
        let u = self.updater();
        let main_id: Vec<String> = monitors
            .iter()
            .filter(|m| m.is_main)
            .map(|m| m.id.clone())
            .collect();
        let pick = {
            let u = u.clone();
            move |v: Pick, cx: &mut gpui::App| {
                let main_id = main_id.clone();
                u(
                    Box::new(move |d| {
                        d.taskbar_monitors = match v {
                            Pick::Main => TaskbarMonitors::Main,
                            Pick::All => TaskbarMonitors::All,
                            // Start from the main taskbar; keep an earlier choice.
                            Pick::Choose => match &d.taskbar_monitors {
                                TaskbarMonitors::Chosen(ids) => {
                                    TaskbarMonitors::Chosen(ids.clone())
                                }
                                _ => TaskbarMonitors::Chosen(main_id),
                            },
                        }
                    }),
                    cx,
                )
            }
        };
        let mut col = div().flex().flex_col().child(block(
            "Monitors",
            Some("Where the display shows. The tray icon itself is only on the main taskbar. Other monitors need Windows' \"Show my taskbar on all displays\". A chosen monitor that is unplugged falls back to the main taskbar until it is back."),
            chips(
                "tbmon",
                &[
                    (Pick::Main, "Main taskbar"),
                    (Pick::All, "All taskbars"),
                    (Pick::Choose, "Choose"),
                ],
                &current,
                t,
                pick,
            ),
            t,
        ));
        if let TaskbarMonitors::Chosen(chosen) = &d.taskbar_monitors {
            for m in &monitors {
                let id = m.id.clone();
                let on = chosen.contains(&id);
                let u = u.clone();
                let hint = m
                    .taskbar
                    .is_none()
                    .then_some("No taskbar on this monitor right now.");
                col = col.child(row(
                    &m.label(),
                    hint,
                    Switch::new(SharedString::from(format!("tbmon-{}", m.number)))
                        .checked(on)
                        .disabled(m.taskbar.is_none() && !on)
                        .on_click(move |v, _, cx| {
                            let (v, id) = (*v, id.clone());
                            u(
                                Box::new(move |d| {
                                    if let TaskbarMonitors::Chosen(ids) = &mut d.taskbar_monitors {
                                        ids.retain(|x| *x != id);
                                        if v {
                                            ids.push(id);
                                        }
                                    }
                                }),
                                cx,
                            )
                        }),
                    t,
                ));
            }
        }
        col
    }

    /// "‹ Accounts" over a page drilled into from the list (saves open edits, like any page
    /// switch).
    fn back_to_accounts(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().child(
            back_link("back-accounts", "Accounts", t)
                .on_click(cx.listener(|this, _, window, cx| this.go(Page::Accounts, window, cx))),
        )
    }

    /// Accounts: one row per account grouped by provider; a row opens its page. "+ Add accounts"
    /// opens detection / import (owner, 2026-11-07: accounts are no longer menu entries).
    fn accounts_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        struct RowData {
            id: Uuid,
            label: String,
            hidden: bool,
            tag: String,
            tag_color: Hsla,
            source: &'static str,
            health: Health,
        }
        let groups: Vec<(ProviderKind, Vec<RowData>)> = {
            let s = self.state.read(cx);
            let views = crate::hover_card::account_views(s);
            [ProviderKind::Claude, ProviderKind::Codex]
                .into_iter()
                .map(|p| {
                    let rows = views
                        .iter()
                        .filter(|v| v.account.provider == p)
                        .map(|v| {
                            let a = v.account;
                            let (tag, tag_color) = account_tag(s, a);
                            RowData {
                                id: a.id,
                                label: a.label.clone(),
                                hidden: a.hidden,
                                tag,
                                tag_color,
                                source: source_hint(a),
                                health: hover::health(v),
                            }
                        })
                        .collect();
                    (p, rows)
                })
                .collect()
        };

        let mut list = div().flex().flex_col();
        let mut n = 0usize;
        for (p, rows) in groups {
            list = list.child(provider_heading(p, t));
            if rows.is_empty() {
                list = list.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(t.faint)
                        .child("No accounts"),
                );
            }
            let count = rows.len();
            let mut g = div().flex().flex_col();
            for r in rows {
                let id = r.id;
                let (status, status_color) = match r.health {
                    Health::Fresh => ("Fresh", t.ok),
                    Health::NoReading | Health::Benched => {
                        (r.health.label().unwrap_or_default(), t.muted)
                    }
                    h => (h.label().unwrap_or_default(), t.warn),
                };
                g = g.child(
                    div()
                        .id(("acc-row", n))
                        .flex()
                        .items_center()
                        .gap_3()
                        .mx(px(-12.0))
                        .px_3()
                        .py(px(10.0))
                        .rounded(px(8.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.surface))
                        .active(|s| s.bg(t.surface_hover))
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_w_0()
                                .items_center()
                                .gap_3()
                                .when(r.hidden, |d| d.opacity(0.55))
                                .child(tag_chip(&r.tag, r.tag_color))
                                .child(
                                    div()
                                        .min_w_0()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .clip("acct-row", SharedString::from(r.label)),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(t.muted)
                                        .child(r.source),
                                )
                                .when(r.hidden, |d| d.child(tag("Hidden", t.muted, t))),
                        )
                        .child(tag(status, status_color, t))
                        .child(icon(Icon::ChevronRight, 14.0, t.faint))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.go(Page::Account(id), window, cx)
                        })),
                );
                n += 1;
            }
            list = list.child(crate::format::capped(
                &format!("accounts-{p:?}"),
                count,
                52.0,
                g,
            ));
        }

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().child(heading("Accounts")))
                    .child(
                        button("add-accounts", "Add accounts", t, true)
                            .child(icon(Icon::Plus, 12.0, gpui::white()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.go(Page::AddAccounts, window, cx)
                            })),
                    ),
            )
            .child(div().text_xs().text_color(t.muted).child(
                "Hub logins appear by themselves; CLI profiles on this PC are added with Add accounts. Click an account for its name, tag, group, billing date, sign-in and removal.",
            ))
            .child(list)
    }

    fn account_page(&self, id: Uuid, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let s = self.state.read(cx);
        let Some(acc) = s.account(id).cloned() else {
            return div().child("Account not found.").into_any_element();
        };
        let (tag_text, tag_color) = account_tag(s, &acc);
        let plan = s.settings.plans.get(&id).cloned();
        let favorite = s.display().taskbar_favorites.contains(&id);
        let preferred = s.settings.steering.preferred == Some(id);
        let groups = s.settings.groups.clone();
        let windows = s
            .settings
            .readings
            .get(&id)
            .map(|r| r.windows.clone())
            .unwrap_or_default();
        let source = source_label(&acc);
        let confirming = self.confirm_remove == Some(id);

        // Header: name, then email / provider / plan / source.
        let header = div()
            .flex()
            .flex_col()
            .gap_2()
            .pb_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(tag_chip(&tag_text, tag_color))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .clip("acct-title", acc.label.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(t.muted)
                    .child(acc.email.clone().unwrap_or_else(|| "email unknown".into()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(icon(
                                Icon::Provider(acc.provider),
                                12.0,
                                provider_color(acc.provider, t.muted),
                            ))
                            .child(provider_name(acc.provider)),
                    )
                    .when_some(plan, |d, p| d.child(tag(&p, t.muted, t)))
                    .child(tag(source, t.accent, t)),
            );

        // Group chips.
        let mut group_chips = div().flex().flex_wrap().gap_1();
        for (i, g) in std::iter::once(None)
            .chain(groups.into_iter().map(Some))
            .enumerate()
        {
            let active = acc.group == g;
            let st = self.state.clone();
            let label = g.clone().unwrap_or_else(|| "No group".into());
            group_chips = group_chips.child(button(("grp", i), &label, t, active).on_click(
                move |_, _, cx| {
                    st.update(cx, |s, cx| {
                        s.set_group(id, g.clone());
                        cx.notify();
                    })
                },
            ));
        }
        let group_block = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(group_chips)
            .child(div().w(px(240.0)).child(Input::new(&self.group)));

        // Windows.
        let mut win_list = div().flex().flex_col().gap_1();
        if windows.is_empty() {
            win_list = win_list.child(
                div()
                    .text_xs()
                    .text_color(t.muted)
                    .child("Windows appear after the first successful reading."),
            );
        }
        for (i, w) in windows.iter().enumerate() {
            let hidden = acc.hidden_windows.contains(&w.key);
            let sh = self.state.clone();
            let k2 = w.key.clone();
            win_list = win_list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .when(hidden, |d| d.text_color(t.faint))
                            .child(w.label.clone()),
                    )
                    .child(
                        button(
                            ("hidew", i),
                            if hidden { "Hidden" } else { "Hide" },
                            t,
                            hidden,
                        )
                        .on_click(move |_, _, cx| {
                            sh.update(cx, |s, cx| {
                                s.toggle_window_hidden(id, &k2);
                                cx.notify();
                            })
                        }),
                    ),
            );
        }

        // Order / hide / remove.
        let (st_up, st_down, st_hide) =
            (self.state.clone(), self.state.clone(), self.state.clone());
        let order = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                icon_button("mv-up", Icon::ArrowUp, "Move up", t).on_click(move |_, _, cx| {
                    st_up.update(cx, |s, cx| {
                        s.move_account(id, true);
                        cx.notify();
                    })
                }),
            )
            .child(
                icon_button("mv-down", Icon::ArrowDown, "Move down", t).on_click(
                    move |_, _, cx| {
                        st_down.update(cx, |s, cx| {
                            s.move_account(id, false);
                            cx.notify();
                        })
                    },
                ),
            )
            .child(
                icon_button(
                    "hide-acc",
                    if acc.hidden { Icon::Eye } else { Icon::EyeOff },
                    if acc.hidden { "Show" } else { "Hide" },
                    t,
                )
                .on_click(move |_, _, cx| {
                    st_hide.update(cx, |s, cx| {
                        s.toggle_hidden(id);
                        cx.notify();
                    })
                }),
            );
        let removable_hint = if acc.hub.is_some() {
            "It comes back while the hub lists this login."
        } else {
            "The CLI profile and its login stay untouched."
        };
        let remove_btn = if confirming {
            div()
                .id("remove")
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .px_3()
                .py(px(5.0))
                .rounded(px(8.0))
                .text_xs()
                .cursor_pointer()
                .bg(t.danger)
                .text_color(gpui::white())
                .hover(|s| s.opacity(0.9))
                .child(icon(Icon::Trash, 12.0, gpui::white()))
                .child("Click again to remove")
        } else {
            icon_button("remove", Icon::Trash, "Remove", t)
        }
        .on_click(cx.listener(move |this, _, _, cx| {
            if this.confirm_remove == Some(id) {
                this.state.update(cx, |s, cx| {
                    s.remove(id);
                    cx.notify();
                });
                this.confirm_remove = None;
                this.page = Page::Accounts;
            } else {
                this.confirm_remove = Some(id);
            }
            cx.notify();
        }));

        let st_fav = self.state.clone();
        let st_pref = self.state.clone();
        div()
            .flex()
            .flex_col()
            .child(self.back_to_accounts(t, cx))
            .child(header)
            .child(row(
                "Name",
                None,
                div().w(px(240.0)).child(Input::new(&self.rename)),
                t,
            ))
            .child(row(
                "Taskbar tag",
                Some("Up to 3 characters; empty = automatic."),
                div().w(px(90.0)).child(Input::new(&self.tag)),
                t,
            ))
            .child(row(
                "Show on taskbar",
                Some("Adds it to the favorites the taskbar shows in Favorites mode."),
                Switch::new(SharedString::from(format!("fav-{id}")))
                    .checked(favorite)
                    .on_click(move |v, _, cx| {
                        let v = *v;
                        st_fav.update(cx, |s, cx| {
                            if s.display().taskbar_favorites.contains(&id) != v {
                                s.toggle_taskbar_favorite(id);
                            }
                            cx.notify();
                        })
                    }),
                t,
            ))
            .when(acc.hub.is_some(), |d| {
                d.child(row(
                    "Preferred for new hub threads",
                    Some("Steering puts this account on top in the hub. Off returns to Auto."),
                    Switch::new(SharedString::from(format!("pref-{id}")))
                        .checked(preferred)
                        .on_click(move |v, _, cx| {
                            let v = *v;
                            st_pref.update(cx, |s, cx| {
                                if (s.settings.steering.preferred == Some(id)) != v {
                                    s.toggle_preferred(id, cx);
                                }
                            })
                        }),
                    t,
                ))
            })
            .child(self.sign_in_block(&acc, t, cx))
            .child(self.billing_block(&acc, t, cx))
            .child(block("Group", None, group_block, t))
            .child(block(
                "Windows",
                Some("Hide a window to drop it from the popover, the taskbar and the hover cards."),
                win_list,
                t,
            ))
            .child(block("Order", None, order, t))
            .child(block(
                "Remove",
                Some(removable_hint),
                div().child(remove_btn),
                t,
            ))
            .into_any_element()
    }

    /// Sign in (opens the CLI login for this profile) and Retry (checks usage now).
    fn sign_in_block(&self, acc: &Account, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let id = acc.id;
        let s = self.state.read(cx);
        let note = s.account_notes.get(&id).cloned();
        let busy = s.signing_in.contains(&id);
        let terminal = match s.settings.tools.terminal {
            subhub_core::settings::Terminal::WindowsTerminal => "Windows Terminal",
            subhub_core::settings::Terminal::PowerShell => "PowerShell",
        };
        let hint = if acc.profile_dir.is_some() {
            format!(
                "Sign in opens the {} login for this profile in {terminal} (change it in General → Advanced). Usage refreshes when it ends. Retry skips failure backoff, never a provider cooldown.",
                provider_name(acc.provider)
            )
        } else {
            "Sign in through the hub: this login belongs to it, SubHub only reads its usage. Retry checks usage now.".to_string()
        };
        let (st_in, st_retry) = (self.state.clone(), self.state.clone());
        let mut actions = div().flex().flex_wrap().items_center().gap_2();
        if acc.profile_dir.is_some() {
            actions = actions.child(
                icon_button(
                    "sign-in",
                    Icon::LogIn,
                    if busy { "Signing in…" } else { "Sign in" },
                    t,
                )
                .on_click(move |_, _, cx| st_in.update(cx, |s, cx| s.sign_in(id, cx))),
            );
        }
        actions = actions.child(
            icon_button("retry-now", Icon::Refresh, "Retry now", t)
                .on_click(move |_, _, cx| st_retry.update(cx, |s, cx| s.refresh_account(id, cx))),
        );
        block(
            "Sign in and retry",
            Some(&hint),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(actions)
                .when_some(note, |d, n| {
                    d.child(div().text_xs().text_color(t.muted).child(n))
                }),
            t,
        )
        .into_any_element()
    }

    /// Optional monthly renewal / access end date with reminders (CONTEXT: Monthly renewal,
    /// Access end date, Billing reminder).
    fn billing_block(&self, acc: &Account, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        use subhub_core::billing;
        let id = acc.id;
        let b = self.state.read(cx).settings.billing.get(&id).cloned();
        let today = AppState::today();
        let status = b
            .as_ref()
            .map(|b| billing::description(b, today))
            .unwrap_or_else(|| "Not tracked".to_string());
        let url = billing::billing_url(acc.provider);
        let me = cx.entity();
        let kind = chips(
            "bill-kind",
            &[(true, "Renewal"), (false, "Access ends")],
            &self.bill_renew,
            t,
            move |renew, cx| {
                me.update(cx, |this, cx| {
                    let before = this.bill_renew;
                    this.bill_renew = renew;
                    let r = this.state.update(cx, |s, cx| {
                        let r = s.set_billing_kind(id, renew);
                        cx.notify();
                        r
                    });
                    if let Err(e) = r {
                        this.bill_renew = before;
                        this.message = Some(e);
                    }
                    cx.notify();
                })
            },
        );
        let st_rem = self.state.clone();
        let mut content = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .when(b.is_none(), |d| d.text_color(t.muted))
                    .child(status),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(130.0)).child(Input::new(&self.bill_date)))
                    .child(kind),
            );
        if let Some(b) = &b {
            let on = b.reminders;
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Switch::new(SharedString::from(format!("bill-rem-{id}")))
                            .checked(on)
                            .on_click(move |v, _, cx| {
                                let v = *v;
                                st_rem.update(cx, |s, cx| {
                                    s.set_billing_reminders(id, v);
                                    cx.notify();
                                })
                            }),
                    )
                    .child(div().text_sm().child("Remind me 7 and 3 days before")),
            );
        }
        let mut buttons = div().flex().flex_wrap().gap_2().child(
            icon_button("open-billing", Icon::ExternalLink, "Open billing", t)
                .tooltip(|window, cx| {
                    gpui_component::tooltip::Tooltip::new(crate::popover::BILLING_HINT)
                        .build(window, cx)
                })
                .on_click(move |_, _, cx| {
                    // Demo mode opens no real billing page.
                    if !crate::demo::on() {
                        cx.open_url(url)
                    }
                }),
        );
        if b.is_some() {
            buttons = buttons.child(
                icon_button("clear-billing", Icon::Close, "Clear", t).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.state.update(cx, |s, cx| {
                            s.clear_billing(id);
                            cx.notify();
                        });
                        this.fill_billing(id, window, cx);
                        cx.notify();
                    },
                )),
            );
        }
        block(
            "Monthly billing",
            Some("Optional. Enter the next renewal, or the date paid access ends, from your billing page (YYYY-MM-DD, saved with Enter or when you leave the field). Later monthly dates are estimates; SubHub never changes a subscription. Reminders appear as tray notices while SubHub runs."),
            content.child(buttons),
            t,
        )
        .into_any_element()
    }

    /// General → Advanced (the former Tools page): what SubHub starts for sign-in and launching.
    fn advanced(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.read(cx);
        let terminal = s.settings.tools.terminal;
        let wt = subhub_core::launch::find_windows_terminal();
        let terminal_hint = match (terminal, &wt) {
            (subhub_core::settings::Terminal::PowerShell, _) => {
                "Sign-in opens in a PowerShell window.".to_string()
            }
            (_, Some(_)) => "Windows Terminal found.".to_string(),
            (_, None) => "Windows Terminal not found: PowerShell is used instead.".to_string(),
        };
        let st = self.state.clone();
        let mut page = div()
            .flex()
            .flex_col()
            .child(
                div().pb_1().text_xs().text_color(t.muted).child(
                    "What SubHub starts for sign-in and the Launch menus. Both CLIs are detected automatically; set a path only when that fails.",
                ),
            )
            .child(row(
                "Sign-in terminal",
                Some(&terminal_hint),
                chips(
                    "terminal",
                    &[
                        (
                            subhub_core::settings::Terminal::WindowsTerminal,
                            "Windows Terminal",
                        ),
                        (subhub_core::settings::Terminal::PowerShell, "PowerShell"),
                    ],
                    &terminal,
                    t,
                    move |v, cx| {
                        st.update(cx, |s, cx| {
                            s.set_terminal(v);
                            cx.notify();
                        })
                    },
                ),
                t,
            ));
        for (provider, input) in [
            (ProviderKind::Claude, &self.claude_path),
            (ProviderKind::Codex, &self.codex_path),
        ] {
            let found = s.cli_path(provider);
            let (status, color) = match &found {
                Some(p) => (format!("Found: {}", crate::demo::show_path(p)), t.ok),
                None => (
                    "Not found. Install the CLI or enter its path.".to_string(),
                    t.warn,
                ),
            };
            page = page.child(block(
                &format!("{} CLI", provider_name(provider)),
                Some("Path to the executable (.exe, .cmd or .bat); saved with Enter or when you leave the field."),
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().w_full().max_w(px(460.0)).child(Input::new(input)))
                    .child(div().text_xs().text_color(color).child(status)),
                t,
            ));
        }
        page
    }

    /// Projects: saved local folders shared by all accounts, in the order of the Launch menus.
    /// Flat rows (label + path), reorder, Show/Hide, rename, Remove (only the shortcut).
    fn projects_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.state.read(cx).settings.projects.clone();
        let last = list.len().saturating_sub(1);
        let mut rows = div().flex().flex_col();
        for (i, p) in list.iter().enumerate() {
            let id = p.id;
            let editing = self.editing_project == Some(id);
            let confirming = self.confirm_remove_project == Some(id);
            let missing = !p.directory.is_dir();
            let state = self.state.clone();

            let title: AnyElement = if editing {
                div()
                    .w_full()
                    .max_w(px(320.0))
                    .child(Input::new(&self.project_label))
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .clip("project", SharedString::from(p.label.clone())),
                    )
                    .when(p.hidden, |d| d.child(tag("Hidden", t.muted, t)))
                    .when(missing, |d| d.child(tag("Folder missing", t.warn, t)))
                    .into_any_element()
            };

            let up_state = state.clone();
            let down_state = state.clone();
            let vis_state = state.clone();
            let rm_state = state.clone();
            let hidden = p.hidden;
            let label = p.label.clone();
            let remove_button = if confirming {
                button(("proj-rm", i), "Click again to remove", t, false).text_color(t.danger)
            } else {
                icon_button(("proj-rm", i), Icon::Trash, "Remove", t)
            };
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_3()
                    .border_b_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .when(hidden, |d| d.opacity(0.6))
                            .child(title)
                            .child(div().min_w_0().text_xs().text_color(t.muted).clip(
                                "project-path",
                                SharedString::from(crate::demo::show_path(&p.directory)),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_1()
                            .child(
                                square_button(("proj-up", i), Icon::ArrowUp, "Move up", i > 0, t)
                                    .on_click(move |_, _, cx| {
                                        up_state.update(cx, |s, cx| {
                                            s.move_project(id, true);
                                            cx.notify();
                                        })
                                    }),
                            )
                            .child(
                                square_button(
                                    ("proj-down", i),
                                    Icon::ArrowDown,
                                    "Move down",
                                    i < last,
                                    t,
                                )
                                .on_click(move |_, _, cx| {
                                    down_state.update(cx, |s, cx| {
                                        s.move_project(id, false);
                                        cx.notify();
                                    })
                                }),
                            )
                            .child(
                                icon_button(
                                    ("proj-vis", i),
                                    if hidden { Icon::Eye } else { Icon::EyeOff },
                                    if hidden { "Show" } else { "Hide" },
                                    t,
                                )
                                .on_click(move |_, _, cx| {
                                    vis_state.update(cx, |s, cx| {
                                        s.set_project_hidden(id, !hidden);
                                        cx.notify();
                                    })
                                }),
                            )
                            .child(
                                square_button(("proj-edit", i), Icon::Pencil, "Rename", true, t)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.commit_project_label(cx);
                                        this.editing_project = Some(id);
                                        this.confirm_remove_project = None;
                                        let text = label.clone();
                                        this.project_label
                                            .update(cx, |i, cx| i.set_value(text, window, cx));
                                        this.project_label
                                            .read(cx)
                                            .focus_handle(cx)
                                            .focus(window, cx);
                                        cx.notify();
                                    })),
                            )
                            .child(remove_button.on_click(cx.listener(move |this, _, _, cx| {
                                if this.confirm_remove_project == Some(id) {
                                    this.confirm_remove_project = None;
                                    rm_state.update(cx, |s, cx| {
                                        s.remove_project(id);
                                        cx.notify();
                                    });
                                } else {
                                    this.confirm_remove_project = Some(id);
                                }
                                cx.notify();
                            }))),
                    ),
            );
        }
        if list.is_empty() {
            rows = rows.child(
                div()
                    .py_4()
                    .text_sm()
                    .text_color(t.muted)
                    .child("No saved folders yet."),
            );
        }

        div()
            .flex()
            .flex_col()
            .child(heading("Projects"))
            .child(
                div().pb_3().text_xs().text_color(t.muted).child(
                    "Local folders shared by all accounts. Each account's Launch menu lists the visible ones in this order. Removing a project only removes the shortcut, never the folder.",
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .pb_2()
                    .child(
                        button("proj-add", "Add project", t, true)
                            .child(icon(Icon::FolderPlus, 12.0, gpui::white()))
                            .on_click(cx.listener(|this, _, _, cx| this.pick_project_folders(cx))),
                    )
            )
            .child(crate::format::capped("projects", list.len(), 52.0, rows))
    }

    /// About & updates: what SubHub is, then update mode, version, last check, Check now, the
    /// status with Install, and the settings file location. Messages come from `updater` and are
    /// already safe (no URLs, paths or tokens).
    fn about_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::updater::{Phase, running_version};
        use subhub_core::settings::UpdateMode;
        let (mode, last_check, phase, available, downloaded, error, dev) = {
            let s = self.state.read(cx);
            (
                s.settings.updates.mode,
                s.settings.updates.last_check,
                s.update.phase,
                s.update.available.as_ref().map(|r| r.version),
                s.update.downloaded,
                s.update.error.clone(),
                s.update.dev_build,
            )
        };
        let busy = phase != Phase::Idle;
        let version = running_version();
        let last = match last_check {
            Some(at) => crate::format::ago(at, chrono::Utc::now()),
            None => "never".to_string(),
        };

        let state = self.state.clone();
        let mode_row = row(
            "Update mode",
            Some(
                "Auto checks about every 6 hours, downloads in the background and restarts SubHub when no window is open. Manual only tells you here and waits for Install.",
            ),
            chips(
                "update-mode",
                &[(UpdateMode::Auto, "Auto"), (UpdateMode::Manual, "Manual")],
                &mode,
                t,
                move |m, cx| {
                    state.update(cx, |s, cx| {
                        s.set_update_mode(m);
                        cx.notify();
                    })
                },
            ),
            t,
        );
        let version_row = row(
            "Current version",
            dev.then_some("Development build: updates are off"),
            div().text_sm().child(format!("v{version}")),
            t,
        );
        let check_row = row(
            "Last checked",
            None,
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().text_sm().text_color(t.muted).child(last))
                .child(
                    button(
                        "update-check",
                        if phase == Phase::Checking {
                            "Checking…"
                        } else {
                            "Check now"
                        },
                        t,
                        false,
                    )
                    .when(busy, |d| d.opacity(0.5))
                    .when(!busy, |d| {
                        d.on_click(cx.listener(|this, _, _, cx| {
                            this.state.update(cx, |s, cx| s.check_update(cx));
                        }))
                    }),
                ),
            t,
        );

        // Status line: what is happening, what is available, or the last problem.
        let (status, status_color, hint) = match (phase, available) {
            (Phase::Checking, _) => ("Checking…".to_string(), t.muted, None),
            (Phase::Downloading, v) => (
                format!(
                    "Downloading{}…",
                    v.map(|v| format!(" v{v}")).unwrap_or_default()
                ),
                t.muted,
                None,
            ),
            (Phase::Installing, v) => (
                format!(
                    "Installing{}…",
                    v.map(|v| format!(" v{v}")).unwrap_or_default()
                ),
                t.muted,
                Some("SubHub restarts in a moment."),
            ),
            (Phase::Idle, Some(v)) => {
                let hint = if dev {
                    None
                } else {
                    Some(match (mode, downloaded) {
                        (UpdateMode::Manual, _) => {
                            "Install downloads it, replaces SubHub and restarts it."
                        }
                        (UpdateMode::Auto, true) => {
                            "Downloaded. It installs when no popover, Settings or Statistics window is open and no sign-in is running."
                        }
                        (UpdateMode::Auto, false) => {
                            "It downloads in the background and installs when SubHub is idle."
                        }
                    })
                };
                (format!("Update available: v{v}"), t.accent_text(), hint)
            }
            (Phase::Idle, None) if last_check.is_some() && error.is_none() => {
                ("SubHub is up to date.".to_string(), t.muted, None)
            }
            (Phase::Idle, None) => (String::new(), t.muted, None),
        };
        let can_install = !dev && !busy && available.is_some();
        let install_label = if mode == UpdateMode::Auto {
            "Install now"
        } else {
            "Install"
        };
        let status_row = (!status.is_empty() || error.is_some()).then(|| {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x_4()
                .gap_y_2()
                .py_3()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(240.0))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .when(!status.is_empty(), |d| {
                            d.child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(status_color)
                                    .child(SharedString::from(status.clone())),
                            )
                        })
                        .when_some(hint, |d, h| {
                            d.child(div().text_xs().text_color(t.muted).child(h))
                        })
                        .when_some(error.clone(), |d, e| {
                            d.child(
                                div()
                                    .text_xs()
                                    .text_color(t.danger)
                                    .child(SharedString::from(e)),
                            )
                        }),
                )
                .when(can_install, |d| {
                    d.child(div().flex_none().ml_auto().child(
                        button("update-install", install_label, t, true).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.state.update(cx, |s, cx| s.install_update(cx));
                            },
                        )),
                    ))
                })
        });

        div()
            .flex()
            .flex_col()
            .child(heading("About & updates"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .pb_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(t.muted)
                            .child("Built with Rust + GPUI. Free, open source (MIT), local: no account, backend or telemetry."),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(t.muted)
                            .child("Claude usage uses an undocumented endpoint and can break when the provider changes it."),
                    ),
            )
            .child(version_row)
            .when(crate::demo::on(), |d| {
                d.child(row(
                    "Updates",
                    Some("No checks, downloads or installs in the demo."),
                    div().text_sm().text_color(t.muted).child("Demo mode"),
                    t,
                ))
            })
            .when(!crate::demo::on(), |d| {
                d.child(mode_row)
                    .child(check_row)
                    .when_some(status_row, |d, r| d.child(r))
            })
            .child(block(
                "Settings file",
                Some("Preferences, accounts and the last readings; never keys or tokens."),
                div()
                    .text_xs()
                    .text_color(t.muted)
                    .child(crate::demo::show_path(&subhub_core::settings::default_path())),
                t,
            ))
    }

    /// Add accounts → "Choose a folder manually…": provider, name and profile folder (the old
    /// Add account page), for a profile detection does not find.
    fn manual_form(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let provider = self.add_provider;
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .pb_2()
                    .text_xs()
                    .text_color(t.muted)
                    .child("Adds a CLI profile folder that detection did not find. A login that is already added is linked, never added twice."),
            )
            .child(row(
                "Provider",
                None,
                chips(
                    "addp",
                    &[
                        (ProviderKind::Claude, "Claude"),
                        (ProviderKind::Codex, "Codex"),
                    ],
                    &provider,
                    t,
                    {
                        let me = cx.entity();
                        move |p, cx| {
                            me.update(cx, |this, cx| {
                                this.add_provider = p;
                                cx.notify();
                            })
                        }
                    },
                ),
                t,
            ))
            .child(row(
                "Name",
                None,
                div().w(px(300.0)).child(Input::new(&self.add_label)),
                t,
            ))
            .child(row(
                "Profile folder",
                None,
                div().w(px(300.0)).child(Input::new(&self.add_dir)),
                t,
            ))
            .child(
                div()
                    .pt_4()
                    .child(button("add", "Add account", t, true).on_click(cx.listener(
                        |this, _, window, cx| {
                            let label = this.add_label.read(cx).value().to_string();
                            let dir = this.add_dir.read(cx).value().to_string();
                            let p = this.add_provider;
                            let r = this.state.update(cx, |s, cx| {
                                let r = s.add_profile(p, &label, &dir);
                                cx.notify();
                                r
                            });
                            this.message = Some(match r {
                                Ok(()) => {
                                    this.add_label
                                        .update(cx, |i, cx| i.set_value("", window, cx));
                                    this.add_dir.update(cx, |i, cx| i.set_value("", window, cx));
                                    this.state.update(cx, |s, cx| s.refresh(false, cx));
                                    "Account added.".into()
                                }
                                Err(e) => e,
                            });
                            cx.notify();
                        },
                    ))),
            )
    }

    /// Runs profile detection again and ticks the rows that are new with a known login.
    fn run_detect(&mut self, cx: &mut Context<Self>) {
        self.detected = self.state.read(cx).detect_profiles();
        self.detect_sig = accounts_sig(self.state.read(cx));
        self.picked = self
            .detected
            .iter()
            .filter(|d| d.preselected())
            .map(|d| d.dir.clone())
            .collect();
    }

    /// The account list changed while the page is open (e.g. the first refresh added the hub
    /// logins): statuses are recomputed, ticks on rows that are still importable are kept, rows
    /// that were not importable before get their default tick.
    fn redetect_if_accounts_changed(&mut self, cx: &mut Context<Self>) {
        let sig = accounts_sig(self.state.read(cx));
        if self.page != Page::AddAccounts || sig == self.detect_sig {
            return;
        }
        let before: HashSet<PathBuf> = self
            .detected
            .iter()
            .filter(|d| d.importable())
            .map(|d| d.dir.clone())
            .collect();
        let picked = std::mem::take(&mut self.picked);
        self.detected = self.state.read(cx).detect_profiles();
        self.detect_sig = sig;
        self.picked = self
            .detected
            .iter()
            .filter(|d| {
                d.importable()
                    && if before.contains(&d.dir) {
                        picked.contains(&d.dir)
                    } else {
                        d.preselected()
                    }
            })
            .map(|d| d.dir.clone())
            .collect();
    }

    fn import_picked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picks: Vec<DetectedProfile> = self
            .detected
            .iter()
            .filter(|d| d.importable() && self.picked.contains(&d.dir))
            .cloned()
            .collect();
        if picks.is_empty() {
            return;
        }
        let (report, msg) = self.state.update(cx, |s, cx| {
            let r = s.import_profiles(&picks, cx);
            cx.notify();
            r
        });
        // Show what was imported: the first new (or linked) account's page.
        match report.added.first().or(report.linked.first()) {
            Some(&id) => self.go(Page::Account(id), window, cx),
            None => self.run_detect(cx),
        }
        self.message = Some(msg);
        cx.notify();
    }

    /// Add accounts (from Accounts): CLI profiles found on this PC (with what importing would do)
    /// and the hub's logins, which the hub adds by itself on each refresh; at the bottom of the
    /// CLI tab the manual folder form.
    fn add_accounts_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let importable: Vec<&DetectedProfile> =
            self.detected.iter().filter(|d| d.importable()).collect();
        let n_picked = importable
            .iter()
            .filter(|d| self.picked.contains(&d.dir))
            .count();

        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .pb_2()
            .child(
                button("det-all", "Select all", t, false).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.picked = this
                            .detected
                            .iter()
                            .filter(|d| d.importable())
                            .map(|d| d.dir.clone())
                            .collect();
                        cx.notify();
                    },
                )),
            )
            .child(
                button("det-none", "Select none", t, false).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.picked.clear();
                        cx.notify();
                    },
                )),
            )
            .child(
                icon_button("det-again", Icon::Refresh, "Detect again", t).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.run_detect(cx);
                        cx.notify();
                    },
                )),
            )
            .child(div().flex_1())
            .child(
                button(
                    "det-import",
                    &format!("Import selected ({n_picked})"),
                    t,
                    n_picked > 0,
                )
                .when(n_picked == 0, |d| d.opacity(0.5).cursor_default())
                .on_click(cx.listener(|this, _, window, cx| this.import_picked(window, cx))),
            );

        let mut list = div().flex().flex_col();
        for p in [ProviderKind::Claude, ProviderKind::Codex] {
            list = list.child(provider_heading(p, t));
            let rows: Vec<(usize, &DetectedProfile)> = self
                .detected
                .iter()
                .enumerate()
                .filter(|(_, d)| d.provider == p)
                .collect();
            if rows.is_empty() {
                list = list.child(
                    div()
                        .py_2()
                        .text_xs()
                        .text_color(t.faint)
                        .child("No profiles found"),
                );
            }
            let count = rows.len();
            let mut g = div().flex().flex_col();
            for (i, d) in rows {
                g = g.child(self.detect_row(i, d, t, cx));
            }
            list = list.child(crate::format::capped(
                &format!("detected-{p:?}"),
                count,
                56.0,
                g,
            ));
        }

        let tab = self.detect_tab;
        let tabs = div().pb_3().child(chips(
            "det-tab",
            &[(DetectTab::Cli, "CLI profiles"), (DetectTab::Hub, "Hub")],
            &tab,
            t,
            {
                let me = cx.entity();
                move |v, cx| {
                    me.update(cx, |this, cx| {
                        this.detect_tab = v;
                        cx.notify();
                    })
                }
            },
        ));
        let page = div()
            .flex()
            .flex_col()
            .child(self.back_to_accounts(t, cx))
            .child(heading("Add accounts"))
            .child(tabs);
        if tab == DetectTab::Hub {
            return page
                .child(div().pb_1().text_xs().text_color(t.muted).child(
                    "Logins in your CLIProxyAPI hub. The hub adds them by itself on each \
                     refresh; a login that also has a CLI profile stays one account.",
                ))
                .child(self.detect_hub_block(t, cx));
        }
        page.child(div().pb_3().text_xs().text_color(t.muted).child(
            "CLI profiles found in your user folder (.claude*, .codex*), in \
                 CLAUDE_CONFIG_DIR and CODEX_HOME, and in .config\\claude. Only the \
                 login email is read; tokens are never touched. A login is never added \
                 twice.",
        ))
        .child(toolbar)
        .child(list)
        .child(
            div().pt_4().child(
                disclosure(
                    "manual-add",
                    "Choose a folder manually…",
                    "Provider, name and profile folder",
                    self.manual_add,
                    t,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.manual_add = !this.manual_add;
                    cx.notify();
                })),
            ),
        )
        .when(self.manual_add, |d| d.child(self.manual_form(t, cx)))
    }

    fn detect_row(
        &self,
        i: usize,
        d: &DetectedProfile,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let importable = d.importable();
        let checked = importable && self.picked.contains(&d.dir);
        let folder = d
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let title = d.email.clone().unwrap_or(folder);
        let path = crate::demo::show_path(&d.dir);
        let path = path.strip_prefix(r"\\?\").unwrap_or(&path).to_string();
        // Importable rows carry no tag: whether the import makes a new account or links the
        // profile to the account that already has this login (e.g. from the hub) is internal
        // (owner, 2026-11-07). Only rows that cannot be imported say why.
        let status = match &d.status {
            DetectStatus::New | DetectStatus::Joins { .. } => None,
            DetectStatus::Added { label, .. } => Some(format!("Already added as {label}")),
            DetectStatus::SameLogin { dir } => {
                let name = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Some(format!("Same login as {name}"))
            }
        };
        // Informed, not asked (owner, 2026-11-07): the login is also in the hub; importing makes
        // no second account, it adds this folder as the account's CLI source.
        let in_hub = match &d.status {
            DetectStatus::Joins { account, .. } | DetectStatus::Added { account, .. } => self
                .state
                .read(cx)
                .account(*account)
                .is_some_and(|a| a.hub.is_some()),
            _ => false,
        };
        let dir = d.dir.clone();
        let control: AnyElement = if importable {
            Checkbox::new(("det-pick", i))
                .checked(checked)
                .on_click(cx.listener(move |this, v: &bool, _, cx| {
                    if *v {
                        this.picked.insert(dir.clone());
                    } else {
                        this.picked.remove(&dir);
                    }
                    cx.notify();
                }))
                .into_any_element()
        } else {
            div().size(px(16.0)).into_any_element()
        };
        div()
            .flex()
            .items_center()
            .gap_3()
            .py_3()
            .border_b_1()
            .border_color(t.border)
            .when(!importable, |d| d.opacity(0.6))
            .child(div().flex_none().child(control))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .min_w_0()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .clip("item-title", SharedString::from(title)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_xs()
                            .text_color(t.muted)
                            .clip("item-path", SharedString::from(path)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap_1()
                    .when(d.email.is_none(), |r| {
                        r.child(tag("No login found", t.warn, t))
                    })
                    .when(in_hub, |r| r.child(tag("Also in the hub", t.muted, t)))
                    .when_some(status, |r, s| r.child(tag(&s, t.muted, t))),
            )
    }

    /// Hub logins are read-only here: the hub adds them on every refresh.
    fn detect_hub_block(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.read(cx);
        let line = match &s.hub {
            HubStatus::Ok(n) => format!(
                "{n} login{} in the hub, added automatically on each refresh.",
                if *n == 1 { "" } else { "s" }
            ),
            HubStatus::Off => "The hub is off.".into(),
            HubStatus::NoKey => "The hub is not set up yet (no management key).".into(),
            HubStatus::Unknown => "The hub has not been checked yet.".into(),
            HubStatus::Unreachable => "The hub is not reachable.".into(),
            HubStatus::Unauthorized => "The hub did not accept the management key.".into(),
            HubStatus::Error(e) => format!("Hub problem: {e}"),
        };
        let set_up = !matches!(s.hub, HubStatus::Ok(_));
        let folder_name = |p: &std::path::Path| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| crate::demo::show_path(p))
        };
        // One account per login (owner, 2026-11-07): a hub login that also has a CLI profile is
        // the same account with both sources; the row says which folder, or that one was found
        // but not imported yet.
        let hub_accounts: Vec<(ProviderKind, String, Option<String>, String, bool)> = s
            .settings
            .accounts
            .iter()
            .filter(|a| a.hub.is_some())
            .map(|a| {
                let cli = match &a.profile_dir {
                    Some(dir) => format!("Also as CLI profile {}", folder_name(dir)),
                    None => match self.detected.iter().find(|d| {
                        d.importable()
                            && d.provider == a.provider
                            && d.email
                                .as_deref()
                                .zip(a.email.as_deref())
                                .is_some_and(|(x, y)| x.eq_ignore_ascii_case(y))
                    }) {
                        Some(d) => {
                            format!("CLI profile {} found, not imported", folder_name(&d.dir))
                        }
                        None => "Hub only".into(),
                    },
                };
                (
                    a.provider,
                    a.label.clone(),
                    a.email.clone(),
                    cli,
                    a.hub.as_ref().is_some_and(|h| h.benched),
                )
            })
            .collect();
        let hub_count = hub_accounts.len();
        let mut rows = div().flex().flex_col();
        for (p, label, email, status, benched) in hub_accounts {
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_2()
                    .border_b_1()
                    .border_color(t.border)
                    .child(icon(Icon::Provider(p), 12.0, provider_color(p, t.muted)))
                    .child(div().flex_1().min_w_0().text_sm().clip(
                        "found",
                        SharedString::from(match email {
                            Some(e) => format!("{label} · {e}"),
                            None => label,
                        }),
                    ))
                    .when(benched, |r| r.child(tag("Benched", t.warn, t)))
                    .child(tag(&status, t.muted, t)),
            );
        }
        let source = format!("Source: CLIProxyAPI hub at {}", s.settings.hub.base_url);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .text_xs()
                    .text_color(t.muted)
                    .child(SharedString::from(source)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .py_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(t.muted)
                            .child(SharedString::from(line)),
                    )
                    .when(set_up, |d| {
                        d.child(button("det-hub", "Hub settings", t, false).on_click(
                            cx.listener(|this, _, window, cx| this.go(Page::Hub, window, cx)),
                        ))
                    }),
            )
            .child(crate::format::capped("hub-accounts", hub_count, 40.0, rows))
    }

    fn hub(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.read(cx);
        let enabled = s.settings.hub.enabled;
        let (status, color) = match &s.hub {
            HubStatus::Off => ("Off".to_string(), t.muted),
            HubStatus::NoKey => ("No key stored".into(), t.warn),
            HubStatus::Unknown => ("Not checked yet".into(), t.muted),
            HubStatus::Ok(n) => (format!("Connected · {n} logins"), t.ok),
            HubStatus::Unreachable => ("Not reachable".into(), t.warn),
            HubStatus::Unauthorized => ("Key rejected".into(), t.danger),
            HubStatus::Error(e) => (format!("Error: {e}"), t.danger),
        };
        let steering = &s.settings.steering;
        let steer_on = steering.enabled;
        let label_of =
            |id: Option<uuid::Uuid>| id.and_then(|id| s.account(id)).map(|a| a.label.clone());
        let (steer_status, steer_color) = match (&s.steering_error, steer_on) {
            (Some(e), _) => (format!("Not applied: {e}"), t.danger),
            (None, false) => ("Off · hub priorities are as you set them".into(), t.muted),
            (None, true) => match (label_of(steering.preferred), label_of(steering.current)) {
                (Some(p), _) => (
                    format!("Preferred: {p} (switch it off on the account page for Auto)"),
                    t.text,
                ),
                (None, Some(c)) => (format!("Auto: new threads go to {c}"), t.text),
                (None, None) => ("Auto: waiting for fresh readings".into(), t.muted),
            },
        };
        let st = self.state.clone();
        let st_check = self.state.clone();
        let key_input = self.hub_key.clone();
        div()
            .flex()
            .flex_col()
            .child(heading("Hub"))
            .child(div().pb_2().text_xs().text_color(t.muted).child(
                "Reads accounts and usage from your CLIProxyAPI hub so you don't add them by hand. The hub keeps renewing its own logins. The only thing SubHub changes there is account priority, for steering.",
            ))
            .child(row("Use the hub", None, Switch::new("hub-on").checked(enabled).on_click(move |v, _, cx| {
                let v = *v;
                st.update(cx, |s, cx| { s.set_hub_enabled(v); s.refresh(true, cx); cx.notify(); })
            }), t))
            .child(row("Status", None, div().text_sm().text_color(color).child(status), t))
            .child(row(
                "Steer new threads",
                Some("Puts the account whose weekly leftover expires first on top in the hub (Auto), or the one you marked Preferred on its account page. Affects new threads in every hub app (e.g. T3 Code). Off restores the original priorities."),
                Switch::new("steer-on").checked(steer_on).on_click({
                    let st = self.state.clone();
                    move |v, _, cx| {
                        let v = *v;
                        st.update(cx, |s, cx| s.set_steering_enabled(v, cx))
                    }
                }),
                t,
            ))
            .child(row("Steering now", None, div().text_sm().text_color(steer_color).child(steer_status), t))
            .child(row("Address", Some("Local by default; the Tailscale address works too."), div().w(px(260.0)).child(Input::new(&self.hub_url)), t))
            .when(crate::demo::on(), |d| {
                // Saving or removing would change the real Windows Credential Manager.
                d.child(row(
                    "Management key",
                    Some(crate::demo::NOT_AVAILABLE),
                    div()
                        .flex()
                        .gap_2()
                        .opacity(0.5)
                        .child(button("save-key", "Save", t, true))
                        .child(button("clear-key", "Remove", t, false)),
                    t,
                ))
            })
            .when(!crate::demo::on(), |d| d.child(row(
                "Management key",
                Some("Stored in Windows Credential Manager, never in the settings file."),
                div()
                    .flex()
                    .gap_2()
                    .child(div().w(px(180.0)).child(Input::new(&self.hub_key)))
                    .child(button("save-key", "Save", t, true).on_click(cx.listener(move |this, _, window, cx| {
                        let v = key_input.read(cx).value().to_string();
                        this.message = Some(match secrets::set_hub_key(&v) {
                            Ok(()) => "Key saved.".into(),
                            Err(e) => e.to_string(),
                        });
                        key_input.update(cx, |i, cx| i.set_value("", window, cx));
                        st_check.update(cx, |s, cx| s.refresh(true, cx));
                        cx.notify();
                    })))
                    .child(button("clear-key", "Remove", t, false).on_click(cx.listener(|this, _, _, cx| {
                        this.message = Some(match secrets::clear_hub_key() {
                            Ok(()) => "Key removed.".into(),
                            Err(e) => e.to_string(),
                        });
                        this.state.update(cx, |s, cx| s.refresh(true, cx));
                        cx.notify();
                    }))),
                t,
            )))
    }
}

/// What detection statuses depend on: each account's id, provider, email and profile folder.
fn accounts_sig(s: &AppState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for a in &s.settings.accounts {
        (a.id, a.provider, &a.email, &a.profile_dir, &a.label).hash(&mut h);
    }
    h.finish()
}

fn source_label(a: &Account) -> &'static str {
    match (&a.profile_dir, &a.hub) {
        (_, Some(h)) if h.benched => "Benched",
        (Some(_), Some(_)) => "Hub + CLI profile",
        (None, Some(_)) => "Hub",
        (Some(_), None) => "CLI profile",
        (None, None) => "Not connected",
    }
}

/// The short source hint on an account row, in the popover's words (Hub, Local only).
fn source_hint(a: &Account) -> &'static str {
    match (&a.profile_dir, &a.hub) {
        (Some(_), Some(_)) => "Hub + profile",
        (None, Some(_)) => "Hub",
        (Some(_), None) => "Local only",
        (None, None) => "Not connected",
    }
}

/// Provider icon and uppercase name over a group of rows.
fn provider_heading(p: ProviderKind, t: &Tokens) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .pt_4()
        .pb_1()
        .child(icon(Icon::Provider(p), 12.0, provider_color(p, t.muted)))
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.faint)
                .child(SharedString::from(provider_name(p).to_uppercase())),
        )
}
