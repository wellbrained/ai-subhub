# Statistics → Overview (plan)

Status: planned 2026-11-07. Built: step 1 (Core: scan v2, `insights`, demo fixtures; 2026-11-07), step 2 (App: Overview tab; 2026-11-08); steps 3-5 open. CoreHub #470. Mockups: `overview2.html` (page), `warmup.html` (5-hour row) in the session scratch folder; screenshots go to `docs/images/` once built.

## Goal

The popover answers "how much is left now". The Overview answers what a user does with that:

1. How much of what I pay for do I use, overall and per account (per 5-hour window and per week)? Should I downgrade or upgrade?
2. What kind of work takes the usage?
3. When does each plan renew or end?
4. Do I spend my 5-hour windows well across accounts?
5. Concrete tips, e.g. start the first window earlier so it resets when I usually run out.

## Decisions (owner, 2026-11-07)

- Overview is a new first tab of the Statistics window and its default; "Limits now" (today's Limits board) and "Tokens & cost" (today's Usage tab) stay behind it.
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
- **Plan fit** (per account, needs 2 counted cycles): tier factor from the plan (Claude Pro 1, Max 5x 5, Max 20x 20; Codex Plus 1, Pro 10, labelled "about"). Peak on a smaller tier = peak × own factor ÷ smaller factor.
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
3. **Tray nudge** (off by default, Settings → General): when an account's session will run out within ~15 min at its current pace and another account of the provider has a fresh window, one balloon per window.
4. **What-if** for ending plans and suggested downgrades (see Analysis).
5. **Light theme**: all colours from `Tokens`; demo screenshots in both themes before release.

## Build order (one commit each, docs and CHANGELOG with it)

1. **Built (2026-11-07).** Core: scan v2 (task kinds, project, thread, sidechain), `insights` (cycles, windows, plan fit, what-if, rhythm, warm-up suggestion and simulation, tips, collecting) with unit tests; demo fixtures with 4+ weeks of history, limit hits, tool calls and thread titles. Entry point `insights::analyze`; how it was built and the choices made: `app.md`, "Overview analysis".
2. **Built (2026-11-08).** App: Overview tab as default; header, Plans, Renewals, Where usage goes; tips with dismiss; collecting states; both themes. How it was built: `app.md`, "Overview tab".
3. App: 5-hour windows row (explainer, timelines, numbers).
4. Warm-up: settings, scheduler, runner, catch-up, Start a window now, wake task + `--warm-up`, demo-safe.
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
