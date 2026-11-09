# Statistics → Overview (plan)

Status: planned 2026-11-07. Built: step 1 (Core: scan v2, `insights`, demo fixtures; 2026-11-07), step 2 (App: Overview tab; 2026-11-08), step 3 (App: 5-hour windows row; 2026-11-08), step 4 (Warm-up; 2026-11-08), step 5 (Tray nudge; 2026-11-08). CoreHub #470. Mockups: `overview2.html` (page), `warmup.html` (5-hour row) in the session scratch folder; screenshots go to `docs/images/` once built.

## Goal

The popover answers "how much is left now". The Overview answers what a user does with that:

1. How much of what I pay for do I use, overall and per account (per 5-hour window and per week)? Should I downgrade or upgrade?
2. What kind of work takes the usage?
3. When does each plan renew or end?
4. Do I spend my 5-hour windows well across accounts?
5. Concrete tips, e.g. start the first window earlier so it resets when I usually run out.

## Decisions (owner, 2026-11-07)

- Overview is a new first tab of the Statistics window and its default; "Limits now" (today's Limits board) and "Tokens & cost" (today's Usage tab) stay behind it. *Changed 2026-11-08:* Statistics and Settings became one main window with a sidebar (see "Main window" below); the topics are pages, not expanding rows.
- Calm layout, everything reachable without scrolling through charts: one verdict sentence, four numbers, the top tip ("N more tips"), then one row per topic that states its answer in words and expands (one open at a time). No cards; space, headings and hairlines only. Messages carry a type icon: tip (lightbulb/clock), danger (triangle), good (check), info (i), saving ($).
- "Where usage goes" splits usage by task kind (from the tools each step used), main thread vs subagents, plus Project, Thread and Model views.
- Warm-up becomes a real feature that runs by itself; the 5-hour row explains how windows and resets work and lets the user set their own start.
- All five extras are in: collecting-data states, dismissible tips, an optional tray nudge, what-if for plan changes, light-theme check.

Decided in planning (change if wrong): the cut-offs and rules below; warm-up skips hub-only accounts in v1; prices are a fixed table with a date; nothing leaves the PC except the warm-up prompt itself.

## Page

Range switch (7 days, 4 weeks, 3 months) applies to everything except "now" values.

**Header.** Verdict sentence built from the strongest finding ("You use about half of what you pay for. Gmail mostly sits idle while …"). Numbers: weekly limits used (avg over plans, peak), 5-hour limit hits per week, time waited for resets, days to the next renewal. Top tip with its action and "N more tips".

**Rows** (collapsed line = the answer):

| Row | Collapsed | Expanded |
| --- | --- | --- |
| Plans | e.g. "Gmail could drop to Max 5x and save ~$100 a month · Gmail Tavora is at its limit", verdict pills | Per account: now (5 h / week bars), weekly used per cycle (bars, avg, peak), 5-hour hits per week, plan fit verdict + one line, what-if |
| 5-hour windows | waited time, warm-up state | Explainer + timeline (today vs with warm-up), waited / with warm-up / extra windows, week timeline of windows by account, warm-up controls, "Start a window now" |
| Renewals | next dates | Per account: renews / ends, days left, reminder on/off, link to the account's billing settings |
| Where usage goes | top two task kinds and projects | Task / Project / Thread / Model switch; bars split main vs subagents; biggest threads; one insight |

## Analysis (`subhub-core/src/insights/`, pure, tested)

Inputs: `readings-history.jsonl` (per refresh: account, window, used %, resets_at), the stats scan (extended, below), settings (accounts, plans, billing), `now`.

- **Weekly cycles**: weekly readings grouped by `resets_at` (minute precision); a cycle's use = its highest used %. A cycle counts when it ended and has readings in at least 3 of its 7 days.
- **5-hour windows**: session readings grouped by `resets_at`; start = reset − 5 h; peak used; hit = first reading at ≥ 99.5 % used; waited = reset − hit. "Another account was free" = at the hit, a non-benched account of the same provider had a reading within 30 min with ≤ 50 % session used and > 10 % weekly left. Windows with no reading in their last 2 hours are "unknown end" (PC off), drawn as gaps, never as hits.
- **Plan fit** (per account, needs 2 counted cycles): tier factor from the plan (Claude Pro 1, Max 5x 5, Max 20x 20; Codex Plus 1, Pro 10, labelled "about"). Copilot, Cursor and Antigravity (2026-11-09, own rows only, monthly cycles, Antigravity by capacity only): `app.md`, "Plan fit for the app logins". Peak on a smaller tier = peak × own factor ÷ smaller factor.
  - *Unused*: avg < 5 %.
  - *Downgrade*: the next smaller tier would keep the peak at ≤ 90 % and the account had ≤ 1 hit a week.
  - *At the limit*: peak ≥ 95 % or ≥ 3 hits a week; suggests the next tier when there is one, else "spread work".
  - *Fits*: everything else.
  - Savings from a price table (USD per month, dated; Claude Pro 20, Max 5x 100, Max 20x 200; Codex Plus 20, Pro 200).
- **What-if**: an account that ends (access end date) or a suggested downgrade moves its average use (in Pro units: avg % × factor) onto the other accounts of the provider in proportion to their free room; shows each one's resulting average ("Gmail would be at ~84 %").
- **Work rhythm**: per weekday, the earliest window start (from window starts, falling back to the first log activity); usual start = median over the last 10 workdays; usual first hit = median (hit − start) of first windows.
- **Warm-up suggestion**: when the first window hit its limit on ≥ 2 of the last 5 workdays: time T so that T + 5 h ≈ usual start + usual first hit, rounded down to 15 min, never more than 4 h before the usual start. Simulation: for each past workday, waiting with the reset moved to T + 5 h (first window only), summed; shown as an estimate.
- **Task kinds** (per assistant step, value split evenly across its tool calls; weight = API value of the step, so models compare):

  | Kind | Tools |
  | --- | --- |
  | Writing code | Edit, Write, MultiEdit, NotebookEdit; Codex `apply_patch` |
  | Builds & tests | Bash / PowerShell / Codex `shell` whose command starts with a build or test tool (cargo, npm, pnpm, yarn, dotnet, pytest, go, make, msbuild, …) |
  | Git & pull requests | the same, starting with git or gh; PR tools |
  | Reading & searching | Read, Grep, Glob, LS; other shell commands that only read (cat, ls, rg, Get-Content, …) |
  | Running agents & threads | Agent / Task, SendMessage, delegate and thread tools |
  | Web research | WebSearch, WebFetch |
  | Design previews | preview, render and screenshot tools |
  | Other tools | anything else |
  | Talking & planning | steps without a tool call |

- **Tips**: each tip has a stable id (kind + account), a priority, a text and an optional action (open the row, settings, steering). Kinds: warm-up, switch instead of waiting, downgrade / upgrade, ending plan, unused plan, task insight (e.g. builds & tests > 20 %), expiring weekly leftover.
- **Collecting state**: every result is `Ready(..)` or `Collecting { ready_on }`. Plan fit needs 2 counted cycles, 5-hour analysis 5 workdays of windows; task kinds work at once (logs go back weeks).

## Scan changes (`stats/parse.rs`, `scan.rs`)

- Claude lines: also read `isSidechain`, `cwd`, the `name` of `tool_use` blocks and, for Bash/PowerShell, the first word of `input.command` (only that word is kept); `customTitle` lines give the thread title. Codex: `function_call` names and the first word of `shell` commands, the turn's `cwd`. Message text is still never kept.
- Buckets gain `kind`, `sidechain`, `project` (a saved project folder when `cwd` is inside one, else the folder name) and `thread`; thread titles in a small map. `CACHE_VERSION` 2 (one full re-scan).

## Warm-up

- Settings `warmup { enabled, time, days, accounts, wake_pc }`, default off; edited in the 5-hour row; Settings → General shows its state with a link.
- At the time, per chosen account with a local profile: skip when its current session window is running (reset in the future and used > 0) or its weekly window is used up. Run hidden, 60 s timeout, billing / API-key variables cleared as for Launch:
  - Claude: `claude -p "ok" --model haiku`, `CLAUDE_CONFIG_DIR` = the profile.
  - Codex: `codex exec --skip-git-repo-check "ok"`, `CODEX_HOME` = the profile.
- Refresh the account 30 s later; keep the last run and its result per account (shown in the row). One retry after 5 min on failure, then wait for the next day.
- Missed run (SubHub not running at the time): run on start only within 60 min after the time.
- "Start a window now" runs the same for one account at once.
- Wake the PC (opt-in): a Windows scheduled task "AI SubHub warm-up" with wake-to-run that starts `ai-subhub.exe --warm-up`, a headless mode that runs the due warm-ups and exits (no single-instance mutex, no UI). Turning it off or turning warm-up off deletes the task.
- Hub-only accounts (no local profile): listed but disabled ("needs a CLI profile") in v1.
- Demo: never runs a CLI or creates a task; it pretends and shows "Demo: nothing was started".

## Extras

1. **Collecting data**: rows show "Collecting data · first result around Nov 21" instead of guesses.
2. **Dismiss tips**: "Not for me" hides a tip id (settings `dismissed_tips`); "Show hidden tips" brings them back.
3. **Tray nudge** (off by default, Settings → General): when an account's session will run out within ~15 min at its current pace and another account of the provider has a fresh window, one balloon per window. *Built 2026-11-08, see "Tray nudge" below.*
4. **What-if** for ending plans and suggested downgrades (see Analysis).
5. **Light theme**: all colours from `Tokens`; demo screenshots in both themes before release.

## Build order (one commit each, docs and CHANGELOG with it)

1. **Built (2026-11-07).** Core: scan v2 (task kinds, project, thread, sidechain), `insights` (cycles, windows, plan fit, what-if, rhythm, warm-up suggestion and simulation, tips, collecting) with unit tests; demo fixtures with 4+ weeks of history, limit hits, tool calls and thread titles. Entry point `insights::analyze`; how it was built and the choices made: `app.md`, "Overview analysis".
2. **Built (2026-11-08).** App: Overview tab as default; header, Plans, Renewals, Where usage goes; tips with dismiss; collecting states; both themes. How it was built: `app.md`, "Overview tab".
3. **Built (2026-11-08).** App: 5-hour windows row (explainer, timelines, numbers). How it was built: `app.md`, "Overview tab".
4. **Built (2026-11-08).** Warm-up: settings, scheduler, runner, catch-up, Start a window now, wake task + `--warm-up`, demo-safe. How it was built: `app.md`, "Warm-up".
5. Tray nudge.

Built-step notes (2026-11-07): buckets keep the working folder (`cwd`), and the project is resolved at analysis time (`insights::project_of`), so editing projects needs no re-scan; the shell command word skips a leading `cd <dir> &&`, environment assignments and `sudo` / `&` (still one word kept); Codex subagent steps are not marked yet (no reliable field); a what-if for a suggested downgrade keeps what fits in the smaller tier and moves only the rest (usually nothing, since the downgrade rule keeps the peak under 90 %).

Edge cases to test: no history; history gaps (PC off); an account added mid-cycle; plan unknown; benched hub logins; resets_at moving by seconds between readings; DST change on a warm-up day; a window running at warm-up time; CLI missing; Codex without a 5-hour window; logs without `cwd` or title.

Step 2 notes (2026-11-08), where it differs from the plan or mockups:

- The window opens on the Overview at a fixed 760 px. The one-time height fit to the Tokens & cost content no longer runs (it would resize the window on the first switch to that tab, against "switching never resizes").
- The 5-hour row shows only its line and the numbers (hits, hits a week, waited, times another account was free, usual start / first run-out, warm-up estimate); explainer, timelines and warm-up controls are step 3. The warm-up tip's "Show me how" opens that row.
- "Not for me" sits on every tip (faint); "Show hidden tips (N)" brings all hidden tips back at once (no per-tip restore).
- Benched logins are left out of the verdict and the Plans line (never capacity); their Plans row shows a "Benched" pill and the fit line plus a benched note. Their tips (e.g. "sits unused") stay.
- Two accounts with the same name are told apart by provider ("Personal (Codex)"); the demo has two such pairs.
- Thread rows needed a main kind and active days: `ThreadShare::kind` / `days` and `UsageReport::thread_count` were added to the core.
- The demo seeds every account's plan label at start (the stale account would otherwise have none, and no what-if). Its weekly grid moves with the start time, so the Side project (12 days of history) always gets two counted cycles: the demo shows Unused, not a Collecting state. The Collecting rendering was checked with a temporary 9-day history.
- Statistics title in the bar is "AI SubHub" (mockup); the taskbar title stays "AI SubHub · Statistics".

Step 3 notes (2026-11-08):

- Core: `WarmupSuggestion::example` (`rhythm::example_day`) is the latest simulated workday the warm-up helped (else the latest it ran on): that account's windows of the day as they went, and with the warm-up the warm-up window (wait from the real first hit when it came before the new reset) followed by back-to-back windows while the real day went on; later hits are not guessed. `WindowsReport::week` holds the last 7 local days of windows per counting account with a 5-hour window (`week_from` local midnight 6 days ago, `week_to` local now). All spans are local wall time (`Span`).
- The right column is "Set your own start" (usual start and run-out, "Send any short message at 7:00 with …", the estimate) until step 4 puts the warm-up controls there.
- Waits are drawn as a thin danger-coloured line, not a red fill, so they never blend with a red or pink account colour; windows use the account colour (solid until a hit, light without one), PC-off windows the track colour.
- Plans are now saved in settings (`Settings::plans`), so plan fit and badges work right after a restart; the Plans "Now" bars follow "Show usage as" (% left by default; the column header says which).

Step 4 notes (2026-11-08), where it differs from or adds to the plan:

- Settings `warmup { enabled, time, days, accounts, wake_pc }` as planned; the last runs are not in settings but in `warmup-runs.json` next to them, shared by the app, a dev build and the wake task's headless run. Whoever runs holds the named mutex `LocalAISubHub.WarmUp` and re-reads the log inside it, so an account is never warmed twice; a process that finds the lock taken tries again on its next tick.
- Due: on a chosen day from the time until 60 min later, an account with no run since the time; a failed first try is retried once 5 min later (the headless run waits for it, keeping the PC awake). A skip (window running, weekly used up) counts as done for the day. "Start a window now" runs the chosen accounts at once with the same skips; it is not retried.
- The CLI runs hidden in `%LOCALAPPDATA%AI SubHubwarm-up` (its sessions are logged under that folder, so the scan shows them as their own small project), stderr is read on a thread, a timeout kills the whole process tree (`taskkill /T`: `claude.cmd` runs node under cmd.exe).
- Wake task: Task Scheduler XML (UTF-16) via `schtasks /Create /XML`, weekly on the chosen days, `WakeToRun`, logged-on user without elevation, on battery too, 10 min limit. It is created / updated when the settings change and at start (an update moves the exe), deleted when Wake the PC or the warm-up is turned off. Dev builds never touch it (it would point at the build's exe) and say so; the demo never does. Windows wakes the PC only when wake timers are allowed in the power options (the row says so). `warmup::tests::task_registers_with_task_scheduler` (ignored; run with `--ignored`) registers it under a test name and deletes it.
- The user always decides (owner, 2026-11-08: "I want the feature, doesn't mean I want it starting by itself"): off by default, nothing runs and no task exists until the user confirms the setup dialog. The warm-up tip's "Set it up…" and the row's "Set up warm-up…" open the dialog filled in with the suggestion (time, accounts, weekdays, Wake the PC off); "Turn on" applies it, Cancel / a click outside leaves everything as it was. With the warm-up on, the row shows it in words with the last runs, "Start a window now", "Change…" (same dialog, "Save") and "Turn off".
- UI: the Warm-up column replaces "Set your own start" on the right (that text moved under the numbers on the left and, with the warm-up on, says what it does instead). In the dialog the time steps by 15 min with "Use 7:00" when the suggestion differs; account chips for visible accounts with a 5-hour window, hub-only ones disabled with "needs a CLI profile on this PC"; "Turn on" needs a day and an account. The collapsed row shows "Warm-up 7:00 weekdays" (clock, green) instead of the waited time, and the warm-up tip is not shown while it is on. Settings → General has a Warm-up row with its state and "Set up in Statistics…" (opens the Overview with the row open, `AppState::pending_overview_row`).
- Demo: the warm-up starts off (the suggestion shows, as for a real user); `--demo-open-row warmup-dialog` opens the setup dialog for screenshots. Turned on in the demo, scheduled runs and "Start a window now" only write "Demo: nothing was started".

## Main window (owner, 2026-11-08)

The owner did not like the expanding rows ("accordion", used nowhere else). Of three mockups (topic tabs, sidebar, drill-in pages; `layouts.html`) the sidebar won, and then Settings joined it (`merged.html`): one window "AI SubHub" with a sidebar of three groups.

- **Insights**: Overview (verdict, numbers, tips, one line per topic with a chevron) and one page per topic (Plans, 5-hour windows, Renewals, Where usage goes): title, the answer line with its note, then the details that used to open under the row. Badges: Plans = accounts with a verdict (red with one at its limit, else blue), 5-hour windows = waits while another account was free (red), Renewals = days to a renewal within 14 days.
- **Data**: Limits now, Tokens & cost (the former tabs).
- **Settings**: General, Appearance, Accounts, Hub, Projects, About & updates (the former Settings window's pages, drawn without its own bar and menu).
- Entry points: Statistics (tray, popover) opens it on the statistics side (last page, Overview at first); Settings opens it on General or the page asked for (card menus: the account, Launch: Projects, first start: Add accounts). An open window just switches. Tips, "Account settings…" and Settings → General → Warm-up "Open 5-hour windows" switch pages inside it.
- The title bar shows "AI SubHub · <page>"; the range switch, Copy as image and Rescan only on the statistics side. Default size 1222 x 760 (the statistics content keeps its 1000 px next to the 210 px sidebar); the former one-time height fit of the Statistics window is gone (switching pages never resizes). The close button follows General → Close button. Auto-update idle check: the main window counts as Settings and Statistics open.
- Code: `stats_window.rs` is the main window (`Section`, `sidebar`, `show_stats` / `show_settings`), it owns the `SettingsWindow` entity; `main.rs` `open_main` (`open_stats` / `open_settings` call it). Demo: `--demo-open-row limits|usage` opens the data pages.

## Tray nudge (owner, 2026-11-08)

Mockups `nudge.html` and `nudge-q.html`. The owner chose the plain tray balloon (no toast buttons: those need a registered app identity) and no notice for accounts the hub steers.

- Setting: Settings → General "Tell me before a window runs out", off by default (`settings.nudge.enabled`). Only tells; nothing switches.
- Shows when an account's 5-hour window runs out within the lead (15 min, or the refresh interval when that is longer, so a slow refresh does not miss it) at its current pace and before it resets, and another visible, unbenched, fresh account of the same provider has ≤ 50 % of its 5-hour window used and weekly room. Pace: the readings of this window from the last 45 min when they span ≥ 10 min, else the window's average since it started (needs 30 min).
- Stays quiet: already shown for this window (`settings.nudge.shown`: account → reset time, pruned after the reset, survives restarts), hidden or benched or stale account, no other account with room, the window resets first, and for hub accounts while hub steering is on and the hub answers (the hub moves on by itself).
- Text: title "<account> (<provider>) runs out in about N min", body names the fresh account with its % left and this window's reset time. Clicking any tray balloon (also billing reminders) opens the popover (`NIN_BALLOONUSERCLICK`).
- Code: `subhub-core/src/nudge.rs` (pure, tested), `state.rs` `trail_point` / `check_nudges` after every reading batch; balloons go through `pending_notices`.

## Two topics per page (owner, 2026-11-08)

Five Insights entries were too many. The sidebar now has Overview, **Plans & renewals** (Plans, then Renewals: what you pay for) and **Usage patterns** (5-hour windows, then Where usage goes: how you use it). Each page shows the page title, then per topic its name, the answer line with its note and the details. The Overview keeps one line per topic; each opens the page its topic is on. A page's badge is the first of its topics' badges (Plans before Renewals). Code: `overview::page_name`, `page_topics`, `page_badge`; `ov.open` still holds the topic.

## Cut-off text (owner, 2026-11-08)

Names and paths that can be cut off with "…" show the full text in a tooltip on hover: `format::Clip` (`div().clip(key, text)`), used for account, project, model and thread names, emails, paths and billing lines.

## Lists that grow (owner, 2026-11-08)

Lists that can grow without end show about 5 rows and scroll the rest inside a scroll area with its own scrollbar (headers stay outside): `format::capped(key, count, row_h, list)`, `VISIBLE_ROWS = 5`. Used for the Plans table, Renewals, the 7-day lanes, the model lists in Tokens & cost, and in Settings the accounts per provider, the found CLI profiles, the hub accounts and Projects. Lists that are already cut to a top N (task bars 10, threads 4) need none.

Opening the second topic of a page (Renewals, Where usage goes; from the Overview line, a tip or the sidebar) scrolls its section to the top once it is laid out; switching to another page starts at the top (`stats_window` `seen_open` / `focus`, `overview::Ctx::focus`). Clipped thread names get an index in their tooltip id, so two "Untitled thread" do not share one.

Settings pages use the same column as the Insights pages (owner, 2026-11-08; they had kept the old Settings window's 680 px): 1000 px plus gutter, centred, 48 px in from the left and 60 px from the right. Row hints stay at most 640 px wide so lines remain readable.
