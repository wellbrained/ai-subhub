# AI SubHub

Source: [wellbrained/ai-subhub](https://github.com/wellbrained/ai-subhub) (private repository).

A free, local Windows tray app for checking AI subscription allowances and launching tools with the right account and project. Version **0.7.0**; MIT licensed. Built with C#/.NET 10 and Avalonia 12.

## Run

Extract the portable ZIP and run `AI.SubHub.exe`. No .NET installation or administrator rights are needed. Configuration lives in `%LOCALAPPDATA%\AI SubHub`. Existing settings migrate automatically from the previous storage folder; account profiles stay in place.

1. Open **Settings → Accounts → Add account**, choose Claude or Codex, and name the account.
2. Choose its existing CLI profile folder, or **Create a separate profile** and then use **⋯ → Sign in**. A profile holds one provider login; select the original folder to preserve its keyring access, settings and history. Default folders are `%USERPROFILE%\.claude` and `%USERPROFILE%\.codex`. Environment overrides are honored as initial suggestions.
3. Add folders in **Settings → Projects**. Use ↑/↓ to set their order and **Show** to include or hide each one. Hidden projects remain saved and editable.
4. On an account row, choose **Launch → Project → target**: Windows Terminal or PowerShell for that CLI profile, VS Code for the folder, or the provider's desktop app. **Manage projects…** opens project settings directly.

The main window defaults to 620 × 404 logical pixels. Accounts use compact rows with one line per allowance; additional accounts scroll inside the window. A small **Add account** button is always beside each provider heading. Reset timestamps and full quota labels are available on hover. There is no global project picker: choose a project for each launch.

Settings is a 600 × 320 window with narrow left navigation, flat account/project rows and small edit dialogs. Profile paths are in the edit dialog and tooltips. Sign-in/removal actions are in each row's ⋯ menu. All settings categories use the same compact controls.

Claude Desktop opens the chosen local folder in its Code view and asks for folder confirmation, using Desktop's login. Codex Desktop currently opens the app and asks you to choose the folder there. Desktop/IDE logins remain separate from the selected CLI profile.

Claude chat Projects (chats, instructions and knowledge) are separate from local project folders. Integration with that list is deferred; this version does not sync or import it.

Account names can be changed in **Edit / rename**. **Remove** only disconnects the account from the hub; its provider profile, credentials and history remain intact. Project removal only removes the saved shortcut.

Closing the window hides it. Click the tray icon to reopen; right-click for Refresh and Exit. Opening the app again brings the existing instance forward. Windows startup is opt-in in Settings → General and starts minimized to the tray. If you move the executable after enabling startup, disable and re-enable that setting from its new location.

## Usage and privacy

- Remaining percentages, reset countdowns, last refresh times and explicit unavailable/stale states. Reset times never imply a fresh allowance without a new provider reading.
- Default refresh is five minutes, configurable from 1–60 minutes. Failed requests back off; provider cooldowns also constrain manual refresh. No desktop notifications.
- Claude: authenticated requests to the **undocumented** `api.anthropic.com/api/oauth/usage` endpoint using the selected profile's existing login. This can break after provider changes. Authentication failures ask you to sign in again; the hub never handles refresh tokens or silently updates the CLI.
- Codex: official CLI app-server `account/read` and `account/rateLimits/read`. Reads all returned quota buckets and durations. Requires a CLI that supports these protocol methods and a ChatGPT subscription login; API-key usage is not a subscription allowance.
- Credentials stay in the CLI profile/keyring. Claude's access token is read into memory only for the direct request. Hub settings contain account labels, profile paths, saved folders and cached usage; no copied credentials, telemetry or Hub backend. The CLIs keep their own existing privacy/settings policies.
- Subscription launches clear inherited API-key/cloud-auth environment overrides. Existing profile configuration remains yours; use profiles configured for subscription authentication. CLI executables are detected automatically or configured in Settings → Tools. Windows Terminal is preferred; PowerShell is the fallback.

macOS/Linux, Gemini, a separate ChatGPT launcher, notifications, installers, public releases and automatic updates are outside this first milestone.

## Development

Install the .NET 10 SDK, then run:

```powershell
./build.ps1                 # Release build + tests
./build.ps1 -Task Publish   # Self-contained Windows x64 executable + ZIP
dotnet run --project src/Hub.App -- --demo
```

The build script also recognizes a project-local SDK at `.tools/dotnet/dotnet.exe`. Demo mode uses simulated accounts without provider requests or changes to real settings. For an isolated visual smoke check, run the built executable with `--demo --ui-smoke --snapshot <absolute-png-path>`; it exits after rendering its window.

Architecture and behavior: [feature guide](docs/features/windows-mvp.md). Product language: [CONTEXT.md](CONTEXT.md). Scope decision: [ADR 0001](docs/adr/0001-windows-first.md).

Provider sources: [Codex app-server](https://learn.chatgpt.com/docs/app-server), [Claude authentication](https://code.claude.com/docs/en/authentication), [Claude profile isolation](https://code.claude.com/docs/en/env-vars). Unofficial endpoint implementation references: [Ceiling](https://github.com/btsouth/ceiling), [AI Quota Deck](https://github.com/JoshuaWang2211/ai-quota-deck). Their source code is not copied into this project.

In 0.4, hover the tray icon for each account's lowest remaining allowance and its reset countdown. Stale/unavailable readings stay explicit; long lists end with +N more. Use **Settings → Accounts → ↑/↓** to order accounts within Claude or Codex. That order is saved and used in the main window and tooltip.

In 0.5, small plan tags follow account names in the main window and Accounts settings. They reflect the profile's saved login (for example Pro, Plus or Max 20×). Hover for the freshness explanation; missing or unsupported metadata shows Plan unknown.

Enable **Settings → General → Start with Windows** to launch in the tray at sign-in. This works for every Windows user without CoreHub or administrator rights. Startup is enabled on the owner's PC. CoreHub keeps the project record but no longer autostarts this app; no dev instance is configured.
