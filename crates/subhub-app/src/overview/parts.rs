use super::*;

// ---------- building blocks ----------

pub(super) fn b(s: impl Into<String>) -> (String, bool) {
    (s.into(), true)
}

pub(super) fn m(s: impl Into<String>) -> (String, bool) {
    (s.into(), false)
}

/// Text with bold parts in the text colour; the rest takes the parent's (muted) colour.
pub(super) fn rich(parts: Vec<(String, bool)>, t: &Tokens) -> StyledText {
    let mut text = String::new();
    let mut hl = vec![];
    for (p, bold) in parts {
        let start = text.len();
        text.push_str(&p);
        if bold {
            hl.push((
                start..text.len(),
                HighlightStyle {
                    color: Some(t.text),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..Default::default()
                },
            ));
        }
    }
    StyledText::new(text).with_highlights(hl)
}

pub(super) fn msg_style(k: Msg, t: &Tokens) -> (Icon, Hsla) {
    match k {
        Msg::Tip => (Icon::Lightbulb, t.accent_text()),
        Msg::Clock => (Icon::Clock, t.accent_text()),
        Msg::Danger => (Icon::Warning, t.danger),
        Msg::Good => (Icon::CircleCheck, t.ok),
        Msg::Info => (Icon::Info, t.info),
        Msg::Saving => (Icon::DollarSign, t.ok),
    }
}

/// The tinted type chip of a message.
pub(super) fn chip(k: Msg, size: f32, t: &Tokens) -> gpui::Div {
    let (glyph, color) = msg_style(k, t);
    div()
        .size(px(size))
        .flex_none()
        .rounded(px((size * 0.3).round()))
        .flex()
        .items_center()
        .justify_center()
        .bg(color.opacity(if t.dark { 0.16 } else { 0.12 }))
        .child(icon(glyph, (size * 0.58).round(), color))
}

pub(super) fn pill(text: impl Into<SharedString>, fg: Hsla, bg: Hsla) -> gpui::Div {
    div()
        .flex_none()
        .whitespace_nowrap()
        .px(px(8.0))
        .py(px(1.0))
        .rounded_full()
        .bg(bg)
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(fg)
        .child(text.into())
}

pub(super) fn fit_pill(v: FitVerdict, t: &Tokens) -> gpui::Div {
    let tint = |c: Hsla| c.opacity(if t.dark { 0.16 } else { 0.12 });
    match v {
        FitVerdict::Fits => pill("Fits", t.ok, tint(t.ok)),
        FitVerdict::Downgrade => pill("Downgrade", t.info, tint(t.info)),
        FitVerdict::AtLimit => pill("At limit", t.danger, tint(t.danger)),
        FitVerdict::Unused => pill("Unused", t.muted, t.surface),
    }
}

pub(super) fn dot(color: Hsla) -> gpui::Div {
    div().size(px(8.0)).flex_none().rounded_full().bg(color)
}

/// A small text link in the accent colour (`faint`: a quiet one, like "Not for me").
pub(super) fn link(
    id: impl Into<gpui::ElementId>,
    text: impl Into<SharedString>,
    faint: bool,
    t: &Tokens,
    on_click: impl Fn(&mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let (color, hover) = if faint {
        (t.faint, t.muted)
    } else {
        (t.accent_text(), t.text)
    };
    div()
        .id(id.into())
        .flex_none()
        .whitespace_nowrap()
        .cursor_pointer()
        .text_size(px(12.5))
        .when(!faint, |d| d.font_weight(FontWeight::SEMIBOLD))
        .text_color(color)
        .hover(move |s| s.text_color(hover))
        .child(text.into())
        .on_click(move |_, _, cx| on_click(cx))
}

/// A horizontal mini bar of `frac` (0..1) on the track.
pub(super) fn mini_bar(frac: f64, color: Hsla, h: f32, t: &Tokens) -> gpui::Div {
    let f = frac.clamp(0.0, 1.0) as f32;
    div()
        .relative()
        .flex_1()
        .h(px(h))
        .rounded(px(h / 2.0))
        .bg(t.track)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(relative(if f > 0.0 { f.max(0.02) } else { 0.0 }))
                .rounded(px(h / 2.0))
                .bg(color),
        )
}

/// Opens the Settings window on a page.
pub type OpenSettings = Rc<dyn Fn(SettingsTarget, &mut App)>;

/// Handlers and data the page needs.
pub struct Ctx<'a> {
    /// The main window's scroll area and the topic it should scroll to once its section is laid
    /// out (opening Renewals or Where usage goes shows that section, not the page top).
    pub scroll: gpui::ScrollHandle,
    pub focus: std::rc::Rc<std::cell::Cell<Option<TipRow>>>,
    pub t: &'a Tokens,
    pub s: &'a AppState,
    pub me: Entity<StatsWindow>,
    pub state: Entity<AppState>,
    pub open_settings: OpenSettings,
}

impl Ctx<'_> {
    /// A click handler that changes the Overview state.
    pub(super) fn upd(
        &self,
        f: impl Fn(&mut OverviewState) + 'static,
    ) -> impl Fn(&mut App) + 'static {
        let me = self.me.clone();
        move |cx: &mut App| {
            me.update(cx, |w, cx| {
                f(w.overview_mut());
                cx.notify();
            })
        }
    }

    pub(super) fn act(&self, a: TipAction) -> impl Fn(&mut App) + 'static {
        let upd = self.upd(move |ov| {
            if let TipAction::OpenRow(r) = a {
                ov.open = Some(r);
            }
        });
        let settings = self.open_settings.clone();
        let setup = open_dialog(self.me.clone(), self.state.clone());
        move |cx: &mut App| match a {
            TipAction::OpenRow(_) => upd(cx),
            TipAction::Steering => settings(SettingsTarget::Hub, cx),
            TipAction::SetUpWarmUp => setup(cx),
        }
    }

    pub(super) fn hidden(&self, id: Uuid) -> bool {
        self.s.account(id).is_some_and(|a| a.hidden)
    }

    pub(super) fn color(&self, id: Uuid) -> Hsla {
        self.s
            .account(id)
            .map_or(self.t.faint, |a| account_tag(self.s, a).1)
    }
}

/// The range switch for the title bar.
pub fn range_switch(ov: &OverviewState, me: Entity<StatsWindow>, t: &Tokens) -> AnyElement {
    let ranges = InsightRange::ALL.map(|r| (r, r.label()));
    seg("ov-range", &ranges, &ov.range, t, move |r, cx| {
        me.update(cx, |w, cx| {
            w.overview_mut().range = r;
            cx.notify();
        })
    })
}
