# AI SubHub

**Your AI subscriptions at a glance, from the Windows tray.**

AI SubHub shows the usage limits of all your **Claude Code** and **Codex** accounts in one place: the 5-hour and weekly windows, how much is left and when each one resets. It also starts the right CLI profile in the right project folder. Free, open source and local, with no account, no backend and no telemetry.

[Download](https://github.com/wellbrained/ai-subhub/releases/latest) · [Changelog](CHANGELOG.md) · [Report a bug](https://github.com/wellbrained/ai-subhub/issues) · [Build from source](#build-from-source) · [MIT license](LICENSE)

<p align="center">
  <img src="docs/images/demo-popover.png" width="420" alt="The AI SubHub popover: Claude accounts with plan badges, session and weekly bars, % left and reset times">
</p>

*All screenshots use demo mode's made-up accounts and usage.*

## What it does

- **All limits in one popover.** Click the tray icon for one card per account, with a bar per window, % left (or % used) and the reset time. Stale readings, cooldowns and sign-in problems are shown, not hidden.
- **Usage on the taskbar.** Optional small badges next to the tray show your favourite accounts at all times. Hover one for details.
- **Several accounts per provider.** Each CLI profile is its own account, with a name, a coloured tag, a plan badge and an optional group. SubHub finds the profiles on your PC and offers to add them.
- **Launch by account and project.** The rocket on a card opens a project folder in Windows Terminal, PowerShell, VS Code or the provider's desktop app, signed in as that account.
- **Usage statistics.** Tokens and an API cost estimate per day, model and provider, read from the CLIs' local session logs, plus a live board of all limits.
- **Renewal reminders.** Optionally enter an account's monthly renewal or access end date to get a reminder 7 and 3 days before.
- **Hub support (optional).** If you run a [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) hub, SubHub lists its logins by itself and can steer new hub threads to the account whose weekly limit expires soonest, or to one you pick.
- **Updates itself** from GitHub Releases, automatically or when you say so, and shows short patch notes afterwards.

<details>
<summary>More screenshots</summary>

![Statistics: work tokens, API estimate, cost by type and a daily chart per provider](docs/images/demo-statistics-usage.png)

![Statistics, Limits tab: a live board of every account's windows](docs/images/demo-statistics-limits.png)

![Settings, Accounts page: accounts per provider with their source and status](docs/images/demo-settings-accounts.png)

</details>

## Get started

Requires **Windows 11** (x64); Windows 10 may work but is untested. macOS and Linux are not supported yet.

1. Download `ai-subhub.exe` from the [latest release](https://github.com/wellbrained/ai-subhub/releases/latest) and put it in a folder you keep, for example `%LOCALAPPDATA%\Programs\AI SubHub`. There is no installer, and updates replace the exe in place.
2. Start it. The exe is not code-signed, so Windows SmartScreen may warn you the first time: **More info → Run anyway**.
3. On the first start, Settings opens on **Add accounts** with the Claude Code and Codex profiles found on your PC (`~\.claude*`, `~\.codex*`, `CLAUDE_CONFIG_DIR`, `CODEX_HOME`). Tick the ones you want. A profile in another place can be added with **Choose a folder manually**.
4. Click the tray icon. If an account says *Sign in needed*, use **Sign in** on its card or settings page; the CLI login opens in a terminal, and the account refreshes when it is done.

**Start with Windows** is opt-in (Settings → General). Closing a window keeps SubHub in the tray; quit it from the tray menu.

### Try it without your accounts

```powershell
ai-subhub.exe --demo
```

Demo mode runs the real app on made-up accounts, usage and projects in a temporary folder. It runs next to your normal SubHub and never touches your accounts, hub, settings or startup entry.

## How usage and credentials work

| Provider | Where usage comes from | Login |
| --- | --- | --- |
| Claude Code | Claude's OAuth usage endpoint (**undocumented**), with the profile's own token | Managed by Claude Code; SubHub only reads the token into memory for the request |
| Codex | The Codex CLI's own app-server (account and rate-limit reads only, no model calls) | Managed by the Codex CLI |
| CLIProxyAPI hub (optional) | The hub's management API; the hub adds the token itself | Your hub's management key, stored in Windows Credential Manager |

- Requests go straight to the providers or your own hub. There is **no SubHub server and no telemetry**. The only other requests are the update check (GitHub) and a public model price list ([LiteLLM](https://github.com/BerriAI/litellm)) for cost estimates.
- SubHub never refreshes, copies or saves provider tokens. Its settings (`%LOCALAPPDATA%\AI SubHub\settings.json`) hold account names, profile paths, project folders, preferences and the last readings.
- A failed reading keeps the last one, marked stale. Missing data is shown as unavailable, never as 0%, and a passed reset time never refills a bar without a new reading.
- Errors back off, and provider rate limits are respected, also by manual refresh.
- Claude's usage endpoint is unofficial and can change without notice. API-key billing is outside the scope of a subscription monitor.

## Build from source

Requires the [Rust toolchain](https://rustup.rs/) (stable, edition 2024) on Windows.

```powershell
git clone https://github.com/wellbrained/ai-subhub.git
cd ai-subhub
cargo test --workspace
cargo run -p subhub-app -- --demo      # the app on made-up data
cargo build --release                  # target\release\ai-subhub.exe
```

Built with Rust and [GPUI](https://www.gpui.rs/) (via `gpui-pre` and `gpui-component`). Provider readers, settings and the rules live in `crates/subhub-core`; the tray, windows and UI in `crates/subhub-app`. Behaviour details: [docs/features/app.md](docs/features/app.md).

## Contributing

Bug reports and focused pull requests are welcome. Please include your Windows version, the provider, the CLI version, steps to reproduce and a redacted error message. Keep tokens, credential files and private profile contents out of issues and screenshots (demo mode helps). Run `cargo test --workspace` and `cargo clippy --workspace --all-targets` before submitting.

AI SubHub is an independent project and is not affiliated with Anthropic or OpenAI.
