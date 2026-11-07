//! Settings window: category menu on the left, one content column on the right (owner rule).
//! Menu: settings pages, then a separated provider section (one page per account), with
//! "+ Add account" and "About" pinned at the bottom.
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::Focusable as _;
use gpui::{
    AnyElement, Context, Entity, FontWeight, Hsla, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, Window, div, prelude::*, px,
};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::switch::Switch;
use subhub_core::detect::{DetectStatus, DetectedProfile};
use subhub_core::model::{Account, ProviderKind};
use subhub_core::secrets;
use subhub_core::settings::{
    Density, DisplaySettings, ResetFormat, TaskbarAccounts, TaskbarMode, TaskbarPosition,
    ThemeMode, TrayMetric, UsageStyle,
};
use uuid::Uuid;

use crate::icons::{Icon, icon, provider_color};
use crate::state::{AppState, HubStatus, SettingsTarget, provider_name};
use crate::taskbar::account_tag;
use crate::theme::{self, Tokens};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    General,
    Display,
    Taskbar,
    Hub,
    Tools,
    Projects,
    Statistics,
    Updates,
    Account(Uuid),
    Add,
    Detect,
    About,
}

const TOP_PAGES: [(Page, &str); 8] = [
    (Page::General, "General"),
    (Page::Display, "Display"),
    (Page::Taskbar, "Taskbar"),
    (Page::Hub, "Hub & steering"),
    (Page::Tools, "Tools"),
    (Page::Projects, "Projects"),
    (Page::Statistics, "Statistics"),
    (Page::Updates, "Updates"),
];

const ACCENTS: [&str; 6] = [
    "#3b82f6", "#8b5cf6", "#14b8a6", "#22c55e", "#f97316", "#ec4899",
];

type Upd = Rc<dyn Fn(Box<dyn FnOnce(&mut DisplaySettings)>, &mut gpui::App)>;

pub struct SettingsWindow {
    state: Entity<AppState>,
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
    // Tools page: CLI path inputs and the text they were last saved with.
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
    /// Detect / Import page: the last detection and the ticked folders.
    detected: Vec<DetectedProfile>,
    picked: HashSet<PathBuf>,
    /// The account list the detection was computed against (`accounts_sig`).
    detect_sig: u64,
    _subs: Vec<Subscription>,
}

impl SettingsWindow {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            Some(SettingsTarget::Projects) => Page::Projects,
            Some(SettingsTarget::Detect) => Page::Detect,
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
                match state.update(cx, |s, _| s.pending_settings_page.take()) {
                    Some(SettingsTarget::Projects) => this.go(Page::Projects, window, cx),
                    Some(SettingsTarget::Detect) => this.go(Page::Detect, window, cx),
                    None => {}
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
            page: start_page,
            detected: vec![],
            picked: HashSet::new(),
            detect_sig: 0,
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
        if this.page == Page::Detect {
            this.run_detect(cx);
        }
        this
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

    /// Saves a CLI path typed on the Tools page (Enter, blur) when it changed.
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
        if page == Page::Detect {
            self.run_detect(cx);
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

/// One menu entry (caller adds the content).
fn nav_item(id: impl Into<gpui::ElementId>, active: bool, t: &Tokens) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py(px(6.0))
        .rounded(px(8.0))
        .text_sm()
        .cursor_pointer()
        .when(active, |d| {
            d.bg(t.bg)
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.text)
        })
        .when(!active, |d| {
            d.text_color(t.muted)
                .hover(|s| s.bg(t.surface_hover).text_color(t.text))
        })
}

fn nav_label(text: &str, t: &Tokens) -> impl IntoElement {
    div()
        .px_3()
        .pt_1()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.faint)
        .child(SharedString::from(text.to_uppercase()))
}

fn divider(t: &Tokens) -> impl IntoElement {
    div().h(px(1.0)).mx_2().my_2().bg(t.border)
}

struct NavAccount {
    id: Uuid,
    label: String,
    hidden: bool,
    tag: String,
    tag_color: Hsla,
}

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let d = self.state.read(cx).display().clone();
        let dark = theme::is_dark(d.theme, cx);
        let t = Tokens::new(dark, &d.accent);

        // Accounts per provider for the menu.
        let providers: Vec<(ProviderKind, Vec<NavAccount>)> = {
            let s = self.state.read(cx);
            [ProviderKind::Claude, ProviderKind::Codex]
                .into_iter()
                .map(|p| {
                    let accs = s
                        .settings
                        .accounts
                        .iter()
                        .filter(|a| a.provider == p)
                        .map(|a| {
                            let (tag, tag_color) = account_tag(s, a);
                            NavAccount {
                                id: a.id,
                                label: a.label.clone(),
                                hidden: a.hidden,
                                tag,
                                tag_color,
                            }
                        })
                        .collect();
                    (p, accs)
                })
                .collect()
        };
        // An open account page whose account vanished falls back to General.
        if let Page::Account(id) = self.page
            && !providers.iter().any(|(_, v)| v.iter().any(|a| a.id == id))
        {
            self.page = Page::General;
        }

        let mut top = div().flex().flex_col().gap(px(2.0));
        for (page, label) in TOP_PAGES {
            top = top.child(
                nav_item(label, self.page == page, &t)
                    .child(label)
                    .on_click(cx.listener(move |this, _, window, cx| this.go(page, window, cx))),
            );
        }

        let mut accounts = div().flex().flex_col().gap(px(2.0));
        let mut n = 0usize;
        for (p, list) in &providers {
            let p = *p;
            accounts = accounts.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .child(icon(Icon::Provider(p), 12.0, provider_color(p, t.muted)))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.faint)
                            .child(SharedString::from(provider_name(p).to_uppercase())),
                    ),
            );
            if list.is_empty() {
                accounts = accounts.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(t.faint)
                        .child("No accounts"),
                );
            }
            for a in list {
                let id = a.id;
                let active = self.page == Page::Account(id);
                accounts = accounts.child(
                    nav_item(("acc-nav", n), active, &t)
                        .when(a.hidden, |d| d.opacity(0.55))
                        .child(tag_chip(&a.tag, a.tag_color))
                        .child(div().flex_1().min_w_0().truncate().child(a.label.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.go(Page::Account(id), window, cx)
                        })),
                );
                n += 1;
            }
        }

        let bottom = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(divider(&t))
            .child(
                nav_item("nav-add", self.page == Page::Add, &t)
                    .child(icon(Icon::Plus, 14.0, t.muted))
                    .child("Add account")
                    .on_click(cx.listener(|this, _, window, cx| this.go(Page::Add, window, cx))),
            )
            .child(
                nav_item("nav-detect", self.page == Page::Detect, &t)
                    .child(icon(Icon::ScanSearch, 14.0, t.muted))
                    .child("Detect / Import")
                    .on_click(cx.listener(|this, _, window, cx| this.go(Page::Detect, window, cx))),
            )
            .child(
                nav_item("nav-about", self.page == Page::About, &t)
                    .child(icon(Icon::Info, 14.0, t.muted))
                    .child("About")
                    .on_click(cx.listener(|this, _, window, cx| this.go(Page::About, window, cx))),
            );

        let nav = div()
            .w(px(232.0))
            .flex_none()
            .flex()
            .flex_col()
            .p_3()
            .bg(t.surface)
            .border_r_1()
            .border_color(t.border)
            .child(
                div()
                    .px_3()
                    .pb_3()
                    .pt_1()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("AI SubHub"),
            )
            .child(top)
            .child(divider(&t))
            .child(nav_label("Accounts", &t))
            .child(
                div()
                    .id("nav-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(accounts),
            )
            .child(bottom);

        let content: AnyElement = match self.page {
            Page::General => self.general(&t, cx).into_any_element(),
            Page::Display => self.display_page(&t, cx).into_any_element(),
            Page::Taskbar => self.taskbar_page(&t, cx).into_any_element(),
            Page::Hub => self.hub(&t, cx).into_any_element(),
            Page::Account(id) => self.account_page(id, &t, cx).into_any_element(),
            Page::Add => self.add_form(&t, cx).into_any_element(),
            Page::Detect => self.detect_page(&t, cx).into_any_element(),
            Page::Tools => self.tools_page(&t, cx).into_any_element(),
            Page::Projects => self.projects_page(&t, cx).into_any_element(),
            Page::Statistics => self.statistics_page(&t, cx).into_any_element(),
            Page::Updates => self.updates_page(&t, cx).into_any_element(),
            Page::About => about(&t).into_any_element(),
        };
        let toast = self.toast(&t, cx);

        div()
            .size_full()
            .flex()
            .bg(t.bg)
            .text_color(t.text)
            .child(nav)
            .child(
                // Content scrolls; a toast floats over its bottom edge (a floating toast).
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
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
                                    .max_w(px(640.0))
                                    .child(content),
                            ),
                    )
                    .when_some(toast, |d, toast| d.child(toast)),
            )
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

// ---------- pages ----------

impl SettingsWindow {
    fn general(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.state.read(cx);
        let minutes = s.settings.refresh_minutes;
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
            .child(row(
                "Close button",
                Some("What ✕ on the popover and this window's close button do."),
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
                Some("Opens SubHub in the tray when you sign in to Windows. Per user, no administrator rights; the switch shows what Windows has registered."),
                Switch::new("startup")
                    .checked(crate::startup::is_enabled())
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
            .child(row(
                "Quit now",
                Some("Stops SubHub, including the tray icon and taskbar display."),
                button("quit", "Quit AI SubHub", t, false).on_click(|_, _, cx| cx.quit()),
                t,
            ))
    }

    fn display_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
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
        let (u1, u2, u3, u4, u5, u6, u7, u8) = (
            u.clone(),
            u.clone(),
            u.clone(),
            u.clone(),
            u.clone(),
            u.clone(),
            u.clone(),
            u.clone(),
        );
        div()
            .flex()
            .flex_col()
            .child(heading("Display"))
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
                    move |v, cx| {
                        u1(Box::new(move |d| d.theme = v), cx);
                        let dark = theme::is_dark(v, cx);
                        theme::apply_component_theme(dark, None, cx);
                    },
                ),
                t,
            ))
            .child(row("Accent color", None, accent_row, t))
            .child(row(
                "Show usage as",
                None,
                chips(
                    "style",
                    &[
                        (UsageStyle::Remaining, "% left"),
                        (UsageStyle::Used, "% used"),
                    ],
                    &d.usage_style,
                    t,
                    move |v, cx| u2(Box::new(move |d| d.usage_style = v), cx),
                ),
                t,
            ))
            .child(row(
                "Running out below",
                Some("Windows at or below this % left move to Running out and turn orange."),
                chips(
                    "thr",
                    &[
                        (10u8, "10%"),
                        (15, "15%"),
                        (20, "20%"),
                        (25, "25%"),
                        (30, "30%"),
                        (40, "40%"),
                    ],
                    &d.running_out_threshold,
                    t,
                    move |v, cx| u3(Box::new(move |d| d.running_out_threshold = v), cx),
                ),
                t,
            ))
            .child(row(
                "Reset times",
                None,
                chips(
                    "reset",
                    &[
                        (ResetFormat::Relative, "In 2d 4h"),
                        (ResetFormat::Exact, "Exact time"),
                    ],
                    &d.reset_format,
                    t,
                    move |v, cx| u4(Box::new(move |d| d.reset_format = v), cx),
                ),
                t,
            ))
            .child(row(
                "Density",
                None,
                chips(
                    "density",
                    &[
                        (Density::Comfortable, "Comfortable"),
                        (Density::Compact, "Compact"),
                    ],
                    &d.density,
                    t,
                    move |v, cx| u5(Box::new(move |d| d.density = v), cx),
                ),
                t,
            ))
            .child(row(
                "Plan badges",
                None,
                Switch::new("plan")
                    .checked(d.show_plan)
                    .on_click(move |v, _, cx| {
                        let v = *v;
                        u6(Box::new(move |d| d.show_plan = v), cx)
                    }),
                t,
            ))
            .child(row(
                "Show emails",
                Some("Next to account names in the popover."),
                Switch::new("email")
                    .checked(d.show_email)
                    .on_click(move |v, _, cx| {
                        let v = *v;
                        u7(Box::new(move |d| d.show_email = v), cx)
                    }),
                t,
            ))
            .child(row(
                "Close popover when clicking elsewhere",
                Some("Off: it stays open until you click the tray icon, press Esc or use the close button."),
                Switch::new("blur-close")
                    .checked(d.close_popover_on_blur)
                    .on_click({
                        let u = u.clone();
                        move |v, _, cx| {
                            let v = *v;
                            u(Box::new(move |d| d.close_popover_on_blur = v), cx)
                        }
                    }),
                t,
            ))
            .child(row(
                "Tray icon shows",
                Some("The ring in the taskbar; pin windows to choose what counts."),
                chips(
                    "tray",
                    &[
                        (TrayMetric::LowestPinned, "Lowest pinned"),
                        (TrayMetric::LowestAll, "Lowest overall"),
                    ],
                    &if matches!(d.tray_metric, TrayMetric::Window { .. }) {
                        TrayMetric::LowestPinned
                    } else {
                        d.tray_metric.clone()
                    },
                    t,
                    move |v, cx| u8(Box::new(move |d| d.tray_metric = v), cx),
                ),
                t,
            ))
            .child(
                div().pt_4().child(
                    button("reset-display", "Reset display settings", t, false)
                        .on_click(move |_, _, cx| u(Box::new(|d| *d = Default::default()), cx)),
                ),
            )
    }

    fn taskbar_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let d = self.state.read(cx).display().clone();
        let u = self.updater();
        let (u1, u2, u3, u4) = (u.clone(), u.clone(), u.clone(), u.clone());
        div()
            .flex()
            .flex_col()
            .child(heading("Taskbar"))
            .child(block(
                "Display",
                Some("Always visible on the taskbar (hidden during fullscreen); click to open, drag to move. Badge: provider, account tag, 5h and weekly. Bars: 5-hour bar + %. Numbers: 5-hour over weekly. Focus: one account big + how many others are ready."),
                chips(
                    "tbm",
                    &[
                        (TaskbarMode::Off, "Off"),
                        (TaskbarMode::Badge, "Badge"),
                        (TaskbarMode::Bars, "Bars"),
                        (TaskbarMode::Numbers, "Numbers"),
                        (TaskbarMode::Focus, "Focus"),
                    ],
                    &d.taskbar_mode,
                    t,
                    move |v, cx| u1(Box::new(move |d| d.taskbar_mode = v), cx),
                ),
                t,
            ))
            .child(row(
                "How many",
                Some("Most accounts the taskbar display shows."),
                chips(
                    "tbc",
                    &[(1u8, "1"), (2, "2"), (3, "3"), (4, "4"), (5, "5")],
                    &d.taskbar_count,
                    t,
                    move |v, cx| u2(Box::new(move |d| d.taskbar_count = v), cx),
                ),
                t,
            ))
            .child(block(
                "Which accounts",
                Some("Favorites: accounts with Show on taskbar switched on (up to 3). Focus: the account new hub threads go to. Auto: that account plus any running low. The popover always shows all."),
                chips(
                    "tba",
                    &[
                        (TaskbarAccounts::Favorites, "Favorites"),
                        (TaskbarAccounts::Focus, "Focus"),
                        (TaskbarAccounts::Auto, "Auto"),
                    ],
                    &d.taskbar_accounts,
                    t,
                    move |v, cx| u3(Box::new(move |d| d.taskbar_accounts = v), cx),
                ),
                t,
            ))
            .child(row(
                "Position",
                Some("Left sits next to the weather/widgets button, right next to the tray icons."),
                chips(
                    "tbp",
                    &[
                        (TaskbarPosition::Left, "Left"),
                        (TaskbarPosition::Right, "Right"),
                    ],
                    &d.taskbar_position,
                    t,
                    move |v, cx| {
                        u4(
                            Box::new(move |d| {
                                d.taskbar_position = v;
                                d.taskbar_offset = None;
                            }),
                            cx,
                        )
                    },
                ),
                t,
            ))
    }

    fn account_page(&self, id: Uuid, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let s = self.state.read(cx);
        let Some(acc) = s.account(id).cloned() else {
            return div().child("Account not found.").into_any_element();
        };
        let (tag_text, tag_color) = account_tag(s, &acc);
        let plan = s.plans.get(&id).cloned();
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
                            .truncate()
                            .child(acc.label.clone()),
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
            let pinned = acc.pinned_windows.contains(&w.key);
            let hidden = acc.hidden_windows.contains(&w.key);
            let (sp, sh) = (self.state.clone(), self.state.clone());
            let (k1, k2) = (w.key.clone(), w.key.clone());
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
                        button(("pin", i), if pinned { "Pinned" } else { "Pin" }, t, pinned)
                            .when(pinned, |d| d.child(icon(Icon::Check, 12.0, gpui::white())))
                            .on_click(move |_, _, cx| {
                                sp.update(cx, |s, cx| {
                                    s.toggle_pin(id, &k1);
                                    cx.notify();
                                })
                            }),
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
                this.page = Page::General;
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
                Some("Pin a window to count it for the tray ring; hide it to drop it from the views."),
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
                "Sign in opens the {} login for this profile in {terminal} (change it in Tools). Usage refreshes when it ends. Retry skips failure backoff, never a provider cooldown.",
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
                .on_click(move |_, _, cx| cx.open_url(url)),
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

    fn tools_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(heading("Tools"))
            .child(
                div().pb_2().text_xs().text_color(t.muted).child(
                    "What SubHub starts for sign-in. Leave a path empty to detect the CLI automatically.",
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
                Some(p) => (format!("Found: {}", p.display()), t.ok),
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
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(SharedString::from(p.label.clone())),
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
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(t.muted)
                                    .child(SharedString::from(p.directory.display().to_string())),
                            ),
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
            .child(rows)
    }

    fn statistics_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let (book, updating, seen) = {
            let s = self.state.read(cx);
            let seen: std::collections::BTreeSet<String> = s
                .stats
                .as_ref()
                .map(|sc| sc.buckets.iter().map(|b| b.model.clone()).collect())
                .unwrap_or_default();
            (s.prices.clone(), s.prices_updating, seen)
        };
        let now = chrono::Utc::now();
        let status = if updating {
            "Updating prices…".to_string()
        } else {
            match (book.fetched_at, &book.error) {
                (Some(at), None) => {
                    format!(
                        "Prices from LiteLLM · updated {}",
                        crate::format::ago(at, now)
                    )
                }
                (Some(at), Some(e)) => format!(
                    "Prices from LiteLLM · updated {} · last update failed: {e}",
                    crate::format::ago(at, now)
                ),
                (None, Some(e)) => format!("Built-in prices · LiteLLM not reachable: {e}"),
                (None, None) => "Built-in prices · not updated yet".to_string(),
            }
        };

        // Groups: publisher, then family; only models seen in the logs, plus a family's current
        // model when none of its models was seen.
        const CURRENT: [&str; 5] = [
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-haiku-5-5",
            "claude-fable-5-1",
            "gpt-6.1-sol",
        ];
        let publisher = |m: &str| -> usize {
            if m.starts_with("claude") {
                0
            } else if m.starts_with("gpt")
                || m.contains("codex")
                || m.starts_with("o1")
                || m.starts_with("o3")
                || m.starts_with("o4")
            {
                1
            } else {
                2
            }
        };
        let family = |m: &str| -> usize {
            match publisher(m) {
                0 => ["opus", "sonnet", "haiku", "fable"]
                    .iter()
                    .position(|f| m.contains(f))
                    .unwrap_or(4),
                1 if m.contains("codex") => 1,
                1 if m.starts_with("gpt") => 0,
                1 => 2,
                _ => 0,
            }
        };
        const FAMILIES: [&[&str]; 3] = [
            &["Opus", "Sonnet", "Haiku", "Fable", "Other"],
            &["GPT", "Codex", "Other"],
            &["Models"],
        ];
        const PUBLISHERS: [&str; 3] = ["Anthropic", "OpenAI", "Other"];
        let mut groups: [Vec<std::collections::BTreeSet<String>>; 3] = [
            vec![Default::default(); 5],
            vec![Default::default(); 3],
            vec![Default::default(); 1],
        ];
        for m in &seen {
            groups[publisher(m)][family(m)].insert(m.clone());
        }
        for m in CURRENT {
            let (p, f) = (publisher(m), family(m));
            if groups[p][f].is_empty() {
                groups[p][f].insert(m.to_string());
            }
        }

        let price_text = |v: f64| {
            let s = format!("{v:.3}");
            let s = s.trim_end_matches('0');
            let s = if s.split('.').nth(1).is_some_and(|d| d.len() >= 2) {
                s.to_string()
            } else {
                format!("{v:.2}")
            };
            format!("${s}")
        };
        let cell = |w: f32| div().w(px(w)).flex_none().text_right().text_sm();
        let head = |text: &'static str, w: f32| {
            div()
                .w(px(w))
                .flex_none()
                .text_right()
                .text_xs()
                .text_color(t.muted)
                .child(text)
        };
        let mut table = div().flex().flex_col();
        for (p, name) in PUBLISHERS.iter().enumerate() {
            if groups[p].iter().all(|g| g.is_empty()) {
                continue;
            }
            table = table.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_4()
                    .pb_1()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.faint)
                            .child(SharedString::from(name.to_uppercase())),
                    )
                    .child(head("Input", 60.0))
                    .child(head("Output", 60.0))
                    .child(head("Cache write", 70.0))
                    .child(head("Cache read", 70.0))
                    .child(div().w(px(96.0)).flex_none()),
            );
            for (fi, models) in groups[p].iter().enumerate() {
                if models.is_empty() {
                    continue;
                }
                table = table.child(
                    div()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .text_color(t.muted)
                        .child(FAMILIES[p][fi]),
                );
                for m in models {
                    let found = book.lookup(m);
                    let mut line = div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .py(px(6.0))
                        .border_b_1()
                        .border_color(t.border)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(SharedString::from(m.clone())),
                        );
                    let (badge, color) = match found {
                        Some((price, src)) => {
                            line = line
                                .child(
                                    cell(60.0).child(SharedString::from(price_text(price.input))),
                                )
                                .child(
                                    cell(60.0).child(SharedString::from(price_text(price.output))),
                                )
                                .child(
                                    cell(70.0)
                                        .child(SharedString::from(price_text(price.cache_write))),
                                )
                                .child(
                                    cell(70.0)
                                        .child(SharedString::from(price_text(price.cache_read))),
                                );
                            match src {
                                subhub_core::stats::PriceSource::LiteLlm => ("LiteLLM", t.accent),
                                subhub_core::stats::PriceSource::BuiltIn => ("built-in", t.muted),
                            }
                        }
                        None => {
                            line = line.child(cell(260.0).text_color(t.muted).child("—"));
                            ("No public price", t.warn)
                        }
                    };
                    table = table.child(
                        line.child(
                            div()
                                .w(px(96.0))
                                .flex_none()
                                .flex()
                                .justify_end()
                                .child(tag(badge, color, t)),
                        ),
                    );
                }
            }
        }
        div()
            .flex()
            .flex_col()
            .child(heading("Statistics"))
            .child(div().pb_2().text_xs().text_color(t.muted).child(
                "Usage statistics are an API estimate: the tokens in your session logs priced per million tokens. A model without a public price shows as Unpriced (counted in tokens, not in cost).",
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pt_3()
                    .child(div().flex_1().text_sm().child(SharedString::from(status)))
                    .child(
                        button(
                            "prices-update",
                            if updating { "Updating…" } else { "Update now" },
                            t,
                            true,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.state.update(cx, |s, cx| s.update_prices(true, cx));
                        })),
                    ),
            )
            .child(div().pt_1().text_xs().text_color(t.muted).child(
                "Prices are read-only and update automatically from LiteLLM's public price list (daily); the built-in table fills any gap. Shown: models found in your logs.",
            ))
            .child(table)
    }

    /// Settings -> Updates: mode, version, last check, Check now, and the status with Install.
    /// Messages come from `updater` and are already safe (no URLs, paths or tokens).
    fn updates_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(heading("Updates"))
            .child(mode_row)
            .child(version_row)
            .child(check_row)
            .when_some(status_row, |d, r| d.child(r))
    }

    fn add_form(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let provider = self.add_provider;
        div()
            .flex()
            .flex_col()
            .child(heading("Add account"))
            .child(
                div()
                    .pb_2()
                    .text_xs()
                    .text_color(t.muted)
                    .child("Adds a CLI profile. Only needed for logins the hub does not hold."),
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
        if self.page != Page::Detect || sig == self.detect_sig {
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

    /// Detect / Import: CLI profiles found on this PC (with what importing would do) and the
    /// hub's logins, which the hub adds by itself on each refresh.
    fn detect_page(&self, t: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
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
            list = list.child(
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
                    ),
            );
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
            for (i, d) in rows {
                list = list.child(self.detect_row(i, d, t, cx));
            }
        }

        div()
            .flex()
            .flex_col()
            .child(heading("Detect / Import"))
            .child(div().pb_3().text_xs().text_color(t.muted).child(
                "CLI profiles found in your user folder (.claude*, .codex*), in \
                     CLAUDE_CONFIG_DIR and CODEX_HOME, and in .config\\claude. Only the \
                     login email is read; tokens are never touched. A login is never added \
                     twice.",
            ))
            .child(toolbar)
            .child(list)
            .when(self.detected.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .pt_3()
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(t.muted)
                                .child("No CLI profiles found. Pick a profile folder yourself:"),
                        )
                        .child(button("det-add", "Add account", t, false).on_click(
                            cx.listener(|this, _, window, cx| this.go(Page::Add, window, cx)),
                        )),
                )
            })
            .child(self.detect_hub_block(t, cx))
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
        let path = d.dir.display().to_string();
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
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(SharedString::from(title)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(t.muted)
                            .child(SharedString::from(path)),
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
        let hub_accounts: Vec<(ProviderKind, String, Option<String>, bool, bool)> = s
            .settings
            .accounts
            .iter()
            .filter(|a| a.hub.is_some())
            .map(|a| {
                (
                    a.provider,
                    a.label.clone(),
                    a.email.clone(),
                    a.profile_dir.is_some(),
                    a.hub.as_ref().is_some_and(|h| h.benched),
                )
            })
            .collect();
        let mut rows = div().flex().flex_col();
        for (p, label, email, with_profile, benched) in hub_accounts {
            let status = if with_profile {
                "Hub + profile"
            } else if benched {
                "Hub · benched"
            } else {
                "Hub · added automatically"
            };
            rows =
                rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .py_2()
                        .border_b_1()
                        .border_color(t.border)
                        .child(icon(Icon::Provider(p), 12.0, provider_color(p, t.muted)))
                        .child(div().flex_1().min_w_0().truncate().text_sm().child(
                            SharedString::from(match email {
                                Some(e) => format!("{label} · {e}"),
                                None => label,
                            }),
                        ))
                        .child(tag(status, t.muted, t)),
                );
        }
        div()
            .flex()
            .flex_col()
            .pt_6()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.faint)
                    .pb_1()
                    .child("HUB"),
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
                        d.child(button("det-hub", "Hub & steering", t, false).on_click(
                            cx.listener(|this, _, window, cx| this.go(Page::Hub, window, cx)),
                        ))
                    }),
            )
            .child(rows)
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
            .child(heading("Hub & steering"))
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
            .child(row(
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
            ))
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

fn about(t: &Tokens) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(heading("About"))
        .child(div().text_sm().child(format!("AI SubHub {}", env!("CARGO_PKG_VERSION"))))
        .child(div().text_xs().text_color(t.muted).child("Built with Rust + GPUI. Free, open source (MIT), local: no account, backend or telemetry."))
        .child(div().text_xs().text_color(t.muted).child("Claude usage uses an undocumented endpoint and can break when the provider changes it."))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .pt_2()
                .child(div().text_sm().child("Settings file"))
                .child(
                    div()
                        .text_xs()
                        .text_color(t.muted)
                        .child(subhub_core::settings::default_path().display().to_string()),
                ),
        )
}
