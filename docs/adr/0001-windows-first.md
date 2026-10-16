# 0001 — Windows-first subscription monitor

Date: 2026-10-15. Status: accepted by Nik after grilling.

## Decision

Build v0.1 as a portable Windows x64 tray app with a compact window, using C#/.NET 10 and Avalonia. Keep provider logic separate from Windows launchers so other desktops remain possible later. Target Nik's daily use first; retain the free/open-source/MIT direction.

Start with Claude Code and Codex, multiple renameable accounts bound to CLI profiles, all returned remaining allowances/reset times, explicit stale/unavailable states and last refresh times. Show provider sections with one account row each. Include shared saved project folders, account-specific Terminal/PowerShell launches and opening a folder in VS Code. Closing hides the window; tray menu exits. Dark default with light/system themes. Settings use left categories and one content column. Windows startup is opt-in. Refresh defaults to five minutes with manual refresh and backoff.

Claude must refresh even while sessions are closed. Nik explicitly accepted using its undocumented authenticated usage endpoint and the risk of provider changes. Codex uses the documented app-server protocol. Login and token refresh stay with provider CLIs; Hub stores no copied credentials. Existing profiles retain their original locations. Account removal preserves profile data.

## Reason and tradeoffs

Refinement accepted 2026-10-15 (v0.3): redesign settings as narrow navigation plus compact flat rows and small editors because the card-based settings still occupied too much screen space. Choose a project inside each account's Launch menu, then its target; remove the global picker. Store a manual project order and a reversible hidden flag so inactive work stays saved without cluttering launch menus. Claude chat Projects are a different concept from local project folders; the owner deferred their integration after reviewing official sync/link capabilities.

Refinement accepted 2026-10-15: product name **AI SubHub**. Replace tall cards with compact allowance rows because three subscriptions consumed too much screen height. Keep a small Add account button beside each provider heading at all times. Launch becomes a target menu including Windows Terminal, PowerShell, VS Code project folder and the matching desktop app; desktop login is independent of CLI profiles. Preserve the original configuration folder and profile locations across the display rename.

A Windows-only first delivery reduces distribution/testing scope while retaining the original cross-platform UI stack. C#/.NET requires a development SDK, but self-contained portable delivery avoids asking users to install a runtime. Avalonia keeps future operating-system support feasible without committing to it now.

Official Claude session telemetry cannot guarantee fresh readings when Claude is idle. The undocumented endpoint enables the hub's primary purpose, at the cost of compatibility maintenance and occasional unavailable states. Errors retain timestamped readings rather than implying zero or fresh capacity.

Notifications, Gemini, a separate ChatGPT entry, installers, auto-updates and public-release polish are deferred. First acceptance is a working, tested local portable build. No commits or publication are authorized by this milestone.

2026-10-15: Owner prioritized tray usage tooltip and account ordering. Preserve provider grouping and use up/down controls within each group, consistent with the compact project manager. Store order in the existing account list; keep tooltip bounded to Windows' supported text size with an explicit omitted-account count.

2026-10-15: Owner requested subscription plan tags immediately after profile/account names. Use saved profile login metadata and disclose its freshness limit, retaining compact rows and an explicit unknown state instead of guessing.
