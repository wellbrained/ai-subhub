# AI SubHub

A hub for seeing remaining AI subscription capacity and opening tools in the intended account and project.

## Language

**Provider**: The AI service whose subscription allowance is shown: Claude or Codex in the first version.

**Account**: A named connection to one provider login. Its name is an editable label chosen by the user, not proof of the provider identity.
_Avoid_: Subscription as a synonym for the editable account entry.

**Profile**: The provider-specific workspace containing a login and its associated settings and history. An account refers to one profile; removing the account leaves the profile intact.

**Allowance window**: A provider-defined period or category of usage with its own remaining capacity and reset time. An account can have several windows.
_Avoid_: Balance, which suggests purchased credit.

**Reading**: An observation of an account's allowance windows at a known time.

**Stale reading**: A previously successful reading whose refresh failed or is overdue. It is last known usage, not a current promise.

**Provider cooldown**: A period during which usage-check retries are blocked after provider throttling. It is separate from chat allowance resets and cannot be bypassed by manual retry or restarting SubHub.

**Failure backoff**: A growing delay before the next automatic retry after a failed usage check. A manual retry can skip ordinary failure backoff, but not a provider cooldown.

**Hub**: The CLIProxyAPI proxy on the hub PC that holds pooled provider logins. SubHub only reads from it through its management API.

**Hub account**: An account discovered from the hub. Its usage is read through the hub, which owns and renews its token.

**Benched account**: A hub account disabled in the hub. Shown, but not counted as available capacity.

**Merged account**: One account known both as a CLI profile and as a hub account, matched by email. Readings come from the hub first, the profile as fallback.

**Project folder**: A saved local folder (label + directory) that an account's Launch menu can open in a terminal, VS Code or the provider's desktop app. Shared by all accounts, kept in the user's own order. Removing one only removes the shortcut, never the folder. Not a Claude chat Project.
_Avoid_: Project as a synonym for a provider chat project.

**Visible project / Hidden project**: A hidden project stays listed and editable in Settings at its place in the order but is left out of the Launch menus; showing it again restores it at that place.

**Profile detection**: Looking through the usual places on this PC for CLI profiles (the provider's default folder, its `-suffix` account folders, and folders named by the provider's own environment variables) and offering to import them. It reads only a profile's login email, never its token.

**Detected profile**: A profile folder found by profile detection, together with what importing it would do: become a new account, join an existing account with the same login, nothing because it is already added, or nothing because another folder already has that login.
_Avoid_: Import as a name for the old one-time import from the C# app, which no longer exists.

**Group**: A user-chosen label such as Work or Personal that groups accounts in the views.

**Pinned window**: An allowance window the user pinned to the top of the Priority view.

**Running out**: An allowance window below the running-out threshold (default 25% left).

**Used up**: An allowance window with nothing left until its reset.

**Popover**: The borderless panel that opens above the tray icon; the main surface of the app.

**Tray ring**: The live-drawn tray icon showing the chosen metric as a ring.

**Monthly renewal**: An optional, manually entered date on which an account's subscription renews each month. Later dates are estimates that keep the original day of month.

**Access end date**: A fixed date entered after canceling, when paid access ends. It never recurs.

**Billing reminder**: A notice 7 and 3 calendar days before the next Monthly renewal or Access end date, shown while SubHub runs, at most once per account, cycle and threshold.

**API estimate**: What the tokens in the local session logs would cost at pay-per-token API prices (built-in, user-editable price table). Not what a subscription charges; shown as "API estimate" next to every cost total.

**Cache savings**: What the cached input (cache reads) would have cost at the uncached input price, minus its cache-read cost.

**Unpriced**: A model without a price. Its tokens are counted, its cost is not.

**Update**: A newer SubHub version published as a GitHub Release. Offered only when newer than the running version, never as a downgrade.

**Update mode**: Auto (check, download in the background, install when idle) or Manual (show "Update available", install on click). Both modes check.

**Idle**: No popover, Settings, Statistics or "Updated" window open, no sign-in running, no popover drag. Auto updates restart SubHub only when idle.

**Development build**: SubHub started from cargo's `target\debug` or `target\release` folder. It can check for updates but never installs one.

**Patch notes**: The short bullets per version in `CHANGELOG.md`, shown once after an update, including skipped versions.
