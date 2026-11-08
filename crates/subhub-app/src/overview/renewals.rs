use super::*;

// ----- Renewals -----

pub(super) fn renewals_line(o: &Overview, t: &Tokens) -> (AnyElement, Option<AnyElement>) {
    let upcoming: Vec<&Renewal> = o.renewals.iter().filter(|r| r.days >= 0).collect();
    let Some(first) = upcoming.first() else {
        return (plain("No renewal or end dates set"), None);
    };
    let mut parts = vec![];
    for (i, r) in upcoming.iter().take(3).enumerate() {
        if i > 0 {
            parts.push(m(" · "));
        }
        parts.push(m(format!(
            "{} {} ",
            label_of(o, r.account),
            if r.auto_renew { "renews" } else { "ends" }
        )));
        parts.push(b(date_text(r.date)));
    }
    let right = div()
        .text_size(px(SMALL))
        .text_color(t.faint)
        .child(days_text(first.days))
        .into_any_element();
    (rich(parts, t).into_any_element(), Some(right))
}

pub(super) fn renewals_content(o: &Overview, cx: &Ctx) -> AnyElement {
    let t = cx.t;
    let mut rows = div().flex().flex_col();
    let mut n = 0;
    for r in o.renewals.iter().filter(|r| !cx.hidden(r.account)) {
        let id = r.account;
        let what = match (r.auto_renew, r.days >= 0) {
            (true, _) => "Renews",
            (false, true) => "Ends",
            (false, false) => "Ended",
        };
        let open = {
            let f = cx.open_settings.clone();
            move |cx: &mut App| f(SettingsTarget::Account(id), cx)
        };
        n += 1;
        rows = rows.child(
            div()
                .flex()
                .items_center()
                .gap(px(C_GAP))
                .py(px(9.0))
                .border_t_1()
                .border_color(t.border.opacity(0.7))
                .text_size(px(BODY))
                .child(
                    div()
                        .w(px(C_NAME))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .child(dot(cx.color(id)))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .clip("acct-head", label_of(o, id)),
                        ),
                )
                .child(
                    div()
                        .w(px(190.0))
                        .flex_none()
                        .flex()
                        .gap(px(6.0))
                        .child(format!("{what} {}", date_text(r.date)))
                        .when(r.estimate, |d| {
                            d.child(div().text_color(t.faint).child("· estimated"))
                        }),
                )
                .child(
                    div()
                        .w(px(100.0))
                        .flex_none()
                        .text_color(t.muted)
                        .child(days_text(r.days)),
                )
                .child(
                    div()
                        .w(px(130.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(px(SMALL))
                        .text_color(t.muted)
                        .child(icon(
                            if r.reminders {
                                Icon::Bell
                            } else {
                                Icon::BellOff
                            },
                            13.0,
                            t.faint,
                        ))
                        .child(if r.reminders {
                            "Reminder on"
                        } else {
                            "Reminder off"
                        }),
                )
                .child(div().flex_1())
                .child(link(
                    SharedString::from(format!("renewal-settings-{id}")),
                    "Account settings…",
                    false,
                    t,
                    open,
                )),
        );
    }
    let mut list = div()
        .flex()
        .flex_col()
        .child(crate::format::capped("renewals", n, 40.0, rows));
    let missing: Vec<String> = o
        .accounts
        .iter()
        .filter(|a| !cx.hidden(a.account) && renewal_of(o, a.account).is_none())
        .map(|a| label_of(o, a.account))
        .collect();
    if !missing.is_empty() {
        list = list.child(
            div()
                .pt(px(10.0))
                .border_t_1()
                .border_color(t.border.opacity(0.7))
                .text_size(px(SMALL))
                .text_color(t.muted)
                .child(format!(
                    "No date set for {}. Dates go in each account's settings.",
                    names(&missing)
                )),
        );
    }
    list.into_any_element()
}
