# Cursor: usage limits and tokens (research)

Date: 2026-11-09. Web sources only. No credential files on this PC were touched.
Everything below about endpoints is **unofficial / reverse-engineered** (Cursor publishes no usage API for individual accounts). "[?]" marks uncertain points. Example values are made up.

## 0. Pricing context (matters for the mapping)

- Cursor's current pricing doc lists plans Free / Plus ($8) / Premium ($30) / Super ($100) / Ultra ($200) / Teams / Enterprise. Each paid plan has ONE pool for Cursor models that **resets weekly**; overage is prepaid credit/top-ups, with a monthly additional-usage cap. Hobby/Pro/Pro+/Ultra/Teams are called "legacy" plans with two monthly pools (Cursor Models, Other Models) plus on-demand. https://cursor.com/docs/account/pricing [?: the fetched page text also mentions "The X Subscription" and grok.com; read it yourself before relying on plan names]
- Consequence seen in trackers: the same account can be **monthly** on `usage-summary` and **weekly** on the newer endpoint (Orca: Unified Ultra, legacy endpoint monthly 2026-10-01..2026-11-01, new endpoint 7-day cycle with a non-midnight timestamp). https://github.com/stablyai/orca/issues/31804
- No rolling 5h window like Claude/Codex was found. Windows are: billing cycle (monthly on legacy, weekly on unified/new plans).

## 1. Credentials on Windows

| Source | Path / key | Notes |
|---|---|---|
| Cursor IDE DB | `%APPDATA%\Cursor\User\globalStorage\state.vscdb` (SQLite, table `ItemTable`, key/value) | keys `cursorAuth/accessToken`, `cursorAuth/refreshToken`, `cursorAuth/cachedEmail`, `cursorAuth/stripeMembershipType`. Windows path: https://openusage.sh/docs/providers/cursor/ ; keys: https://github.com/shaulv/AI-Usage-tracker |
| cursor-agent CLI | `%APPDATA%\Cursor\auth.json` (verified on Windows), plaintext JSON with `accessToken` and `refreshToken` | https://github.com/Leonxlnx/tastecode/pull/2004 , https://github.com/barramee27/crossusage/pull/44 , https://github.com/getpaseo/paseo/pull/9180 . Linux: `$XDG_CONFIG_HOME/cursor/auth.json` (`~/.config/cursor/`); macOS: Keychain item `cursor-access-token`. CLI binary under `%LOCALAPPDATA%\cursor-agent\versions` (tastecode PR). |
| CLI config (no secrets) | `%USERPROFILE%\.cursor\cli-config.json`, override `CURSOR_CONFIG_DIR` | https://cursor.com/docs/cli/reference/configuration (docs do not say where auth is stored) |

**Important change (Cursor 3.2x, ~Oct 2026):** the IDE can store the tokens in `state.vscdb` encrypted: value starts with `cursor-safe-storage:v1:` (Electron safeStorage; on Windows DPAPI + AES-GCM [?: Windows detail only from the crossusage PR, which says it does NOT decrypt]). Then the value is not a JWT and returns 401 `ERROR_NOT_LOGGED_IN`. Trackers fall back to the plaintext CLI `auth.json`. https://github.com/steipete/CodexBar/issues/4755 , https://github.com/barramee27/crossusage/pull/44
- Practical plan: read `state.vscdb` (copy first; WAL-locked while Cursor runs, or open read-only/immutable); if the value lacks the `cursor-safe-storage:v1:` prefix use it; else read `auth.json`; else tell the user to run `agent login`. A stale IDE token can 401 while auth.json works, so retry with the next source on 401. https://github.com/getpaseo/paseo/pull/9180
- Token facts: JWT, `sub` like `auth0|user_01...` / `google-oauth2|user_01...`, `aud` https://cursor.com, `type: session`, weeks to ~60 days lifetime (https://github.com/spancerxing/cliproxyapi-cursor-plugin/pull/2 , crossusage PR). Check `exp`.

**Two auth styles (different values!):**
1. `api2.cursor.sh` (Connect RPC): `Authorization: Bearer <bare JWT>`. The prefixed value gives 401. https://github.com/robinebers/openusage/issues/1551
2. `cursor.com/api/*` (web): cookie `WorkosCursorSessionToken=<userId>%3A%3A<jwt>` where `userId` = part of JWT `sub` after the last `|` (e.g. `user_01ABC...`); `%3A%3A` is URL-encoded `::`. Bare JWT -> 401 `{"error":"not_authenticated"}`. Raw `::` also reported working. https://github.com/robinebers/openusage/issues/1551 , https://github.com/Errr0rr404/usage-monitor/pull/5 , https://github.com/steipete/CodexBar/blob/main/docs/cursor.md

**Refresh:** `POST https://api2.cursor.sh/oauth/token` with `{grant_type:"refresh_token", client_id:<Cursor's public client id>, refresh_token}`, no Authorization header; the response gives a new session token, usually no new refresh token. The client id value was not found in the sources (it is in the Cursor IDE/CLI bundles) [?]; it must be the same one the token was issued under. https://github.com/code-yeongyu/senpi/pull/6155 , https://github.com/code-yeongyu/senpi/issues/2919 , https://github.com/eisbaw/cursor_api_demo . Most trackers do NOT refresh (CodexBar, tastecode) and just ask the user to open Cursor / `agent login`. Recommendation: no refresh in v1; refreshing could also race with the CLI/IDE own rotation [my inference].

## 2. Usage endpoints

### A. Newest, Connect RPC (works for current plans; best choice)
`POST https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage`
Headers: `Authorization: Bearer <jwt>`, `Content-Type: application/json`, `Connect-Protocol-Version: 1`, `Accept: application/json`. Body `{}`. https://github.com/shaulv/AI-Usage-tracker , https://openusage.sh/docs/providers/cursor/
(Protobuf also works: `content-type: application/proto`, empty body; fields 1 billing_cycle_start, 2 billing_cycle_end, 3 plan_usage{2 included_spend, 5 limit, 12 auto_percent_used, 13 api_percent_used, 14 total_percent_used} https://github.com/Leonxlnx/tastecode/pull/2004 ; unverified against live traffic.)

Response (JSON, 64-bit ints are strings):
```json
{
  "billingCycleStart": "1792000000000",
  "billingCycleEnd": "1792604800000",
  "planUsage": {
    "totalSpend": 123, "includedSpend": 123, "bonusSpend": 0, "limit": 2000,
    "autoPercentUsed": 1.1, "apiPercentUsed": 0, "totalPercentUsed": 1.1
  },
  "spendLimitUsage": {"pooledLimit": 0, "pooledUsed": 0, "pooledRemaining": 0, "individualUsed": 0},
  "displayMessage": "You've used 1% of your included usage"
}
```
- Start/end are epoch ms as strings (openusage docs say RFC3339; the web variant below returned ISO) -> parse both. Spend values are cents. Percent fields are 0..100 floats (not fractions). https://github.com/stablyai/orca/issues/31804 , https://github.com/shaulv/AI-Usage-tracker
- Team-billed seats: `planUsage` missing and start == end (`displayThreshold: 100`); use `GetTeams` + `GetTeamSpend {teamId}` (`teamMemberSpend[].spendCents`, `effectivePerUserLimitDollars`/`monthlyLimitDollars`, `nextCycleStart`). https://github.com/getpaseo/paseo/issues/2877
- Sibling RPCs on the same service: `GetPlanInfo` (`planInfo.{planName,price,billingCycleEnd,includedAmountCents}`), `GetHardLimit` (`noUsageBasedAllowed`), `GetUsageLimitPolicyStatus`, `GetAggregatedUsageEvents`, `GetTeamMembers`; REST `GET /auth/full_stripe_profile` (`membershipType`, `isTeamMember`, `teamId`...). https://openusage.sh/docs/providers/cursor/
- The official `agent` CLI itself calls getCurrentPeriodUsage + getHardLimit + getPlanInfo for its usage screen (tastecode PR above).

### B. Web equivalent
`POST https://cursor.com/api/dashboard/get-current-period-usage`, body `{}`, headers `Content-Type: application/json`, `Origin: https://cursor.com` (required on POST, else `{"error":"Invalid origin for state-changing request"}`), cookie as in 1.2. Example response (Orca, unified plan): `billingCycleStart "2026-10-18T06:32:13.504Z"`, `billingCycleEnd "2026-10-25T06:32:13.504Z"`, `planUsage.{totalPercentUsed,autoPercentUsed,apiPercentUsed}`, `displayMessage`; no `used/limit`, no `membershipType`. https://github.com/stablyai/orca/issues/31804 , https://gist.github.com/dmwyatt/1e9359b1862e7cbfe1e754fe4c8db764 (Origin rule). The auth used for this call was not shown in the Orca issue [?].

### C. Legacy/web (still alive, but can be stale)
`GET https://cursor.com/api/usage-summary` (Cookie only, no Origin needed):
```json
{
  "billingCycleStart": "2026-10-01T00:00:00.000Z",
  "billingCycleEnd": "2026-11-01T00:00:00.000Z",
  "membershipType": "pro",
  "limitType": "user", "isUnlimited": false,
  "autoModelSelectedDisplayMessage": "You've used 3% of your included total usage",
  "namedModelSelectedDisplayMessage": "You've used 0% of your included API usage",
  "individualUsage": {
    "plan": {"enabled": true, "used": 600, "limit": 2000, "remaining": 1400,
             "breakdown": {"included": 600, "bonus": 0, "total": 600},
             "autoPercentUsed": 3.0, "apiPercentUsed": 0, "totalPercentUsed": 3.0},
    "onDemand": {"enabled": true, "used": 0, "limit": null, "remaining": null}
  },
  "teamUsage": {"onDemand": {"enabled": true, "used": 0, "limit": 50000, "remaining": 50000}}
}
```
`membershipType` values seen: pro, pro_plus, ultra, free_trial, hobby, free, express, team, business, enterprise, `unified_*` (e.g. `unified_ultra`). Money fields in cents. Enterprise/team seat: `individualUsage.plan` absent, `individualUsage.overall{enabled,used,limit,remaining}` (limit can be null), `teamUsage.pooled`/`onDemand`. Unified accounts can show **0%** here while real usage is on endpoint A/B. Sources: https://gist.github.com/dmwyatt/1e9359b1862e7cbfe1e754fe4c8db764 , https://github.com/getagentseal/codeburn/issues/1546 , https://github.com/Errr0rr404/usage-monitor/pull/5 , https://github.com/stablyai/orca/issues/31804 , https://github.com/steipete/CodexBar/blob/main/docs/cursor.md

Other web endpoints (CodexBar docs): `GET /api/auth/me` (email, id), `GET /api/usage?user=<user_01...>` (legacy request counts: `{"gpt-4":{numRequests,numRequestsTotal,numTokens,maxTokenUsage,maxRequestUsage},"startOfMonth":"..."}`; `maxRequestUsage` null on usage-based plans = no limit), `GET /api/dashboard/v2/usage-status?scope=user` (monthly additional spend for unified personal: `usedCents`, `limitCents`, `periodStart`, `resetsAt`), `POST /api/dashboard/get-sand-usage-status` ("Grok Bot" weekly pool: `usagePercent`, `nextResetTimestampUtc`, `isUnified`, `currentPeriodStart`; needs Origin), team: `/api/dashboard/teams`, `/api/dashboard/get-team-spend`. https://github.com/steipete/CodexBar/blob/main/docs/cursor.md

## 3. Mapping to "% used + reset time"

Recommended fallback chain (what Orca/CodexBar converged on):
1. `GetCurrentPeriodUsage` (A). Primary bar = `planUsage.totalPercentUsed`; optional sub-bars Auto (`autoPercentUsed`, Cursor models) and API (`apiPercentUsed`, third-party models). Reset = `billingCycleEnd`. Window label weekly if end-start is ~7d, monthly otherwise (derive from the span, do not hardcode). If total is missing: `includedSpend/limit`.
2. If `planUsage` is absent and start==end -> team seat: GetTeams/GetTeamSpend, or fall back to C.
3. `usage-summary` (C): `individualUsage.plan.totalPercentUsed`, else `used/limit`, else `individualUsage.overall`; reset = `billingCycleEnd` (monthly). Treat a 0% from here on a `unified_*` membership as untrustworthy (per Orca).
4. Optional extra-spend line: `individualUsage.onDemand.used` (cents) in C, `spendLimitUsage` in A, or `usage-status` v2.
Plan name: `GetPlanInfo.planInfo.planName`, `usage-summary.membershipType`, or IDE DB `cursorAuth/stripeMembershipType`.

## 4. CLIProxyAPI

Not supported upstream: the README provider table has Kimi, OpenAI/Codex, Claude, Gemini (incl. Antigravity), xAI Grok, Meta Muse, Devin, plus OpenAI-compatible upstreams; no Cursor, no cursor-agent. https://github.com/router-for-me/CLIProxyAPI (fetched 2026-11-09). A third-party plugin exists (https://github.com/spancerxing/cliproxyapi-cursor-plugin, not evaluated) and third-party tools (Quotio, CLIProxy Quota Tray) list Cursor quotas, but that is not CLIProxyAPI itself. Verdict: no hub path for Cursor.

## 5. Local token logs

Essentially none usable:
- CLI: `~/.cursor/chats/<hash>/<session>/store.db` (SQLite `blobs`, `meta`; meta has `createdAt`, `lastUsedModel`, `mode`; no token counts) and `~/.cursor/projects/<project>/agent-transcripts/*.jsonl` (only `role` + `message.content`). https://github.com/getagentseal/codeburn/issues/986 , https://github.com/ponbac/token-ledger/issues/21 , https://allaboutcoding.ghinda.com/where-ai-coding-clis-store-session-logs/
- `~/.cursor/ai-tracking/ai-code-tracking.db`: AI-code attribution (model name per code hash), no tokens. https://openusage.sh/docs/providers/cursor/
- IDE `state.vscdb` `cursorDiskKV` (multi-GB, constantly rewritten) might hold a usage-ledger-like counter; keys/fields are undocumented in every source found [?]. https://github.com/AlexanderMakarov/llm-wiki/issues/396
- Per-request model + tokens come from the server:
  - `POST https://cursor.com/api/dashboard/get-filtered-usage-events` (Cookie + `Origin: https://cursor.com`; optional body `{startDate:"<ms>", endDate:"<ms>", page:1, pageSize:100, teamId?, userId?}`; `{}` returns the first 100 events of all time):
```json
{"totalUsageEventsCount": 42,
 "usageEventsDisplay": [{"timestamp":"1792000000000","model":"claude-4.6-opus-high-thinking",
   "kind":"USAGE_EVENT_KIND_USAGE_BASED","requestsCosts":1,"usageBasedCosts":"$1.21","isTokenBasedCall":true,
   "tokenUsage":{"inputTokens":1200,"outputTokens":300,"cacheWriteTokens":0,"totalCents":121},
   "cursorTokenFee":5,"chargedCents":126,"isChargeable":true,"isHeadless":false}]}
```
    Source: https://gist.github.com/dmwyatt/1e9359b1862e7cbfe1e754fe4c8db764 . No `cacheReadTokens` in the gist's event shape [?]; the api2 `GetAggregatedUsageEvents` RPC returns `aggregations[]{modelIntent,inputTokens,outputTokens,cacheWriteTokens,cacheReadTokens,totalCents,tier}` with counts as strings (https://openusage.sh/docs/providers/cursor/). A report says summed `chargedCents` looked ~100x `spendCents` from GetTeamSpend [?] (https://github.com/getpaseo/paseo/issues/2877).
  - Paging etiquette (CodexBar): 1000 per page, max 200 pages, 6h cooldown after a 403. https://github.com/steipete/CodexBar/blob/main/docs/cursor.md
  - Team admins also have an official Admin API (`POST /teams/spend`, API key) only.

So for the "Tokens & cost" page, tokens are only available via the server events API, not a local log parser.

## 6. Risks

- All endpoints are undocumented and shapes changed repeatedly within weeks (usage-summary -> get-current-period-usage; weekly vs monthly; encrypted tokens in 3.27; cookie prefix requirement). Expect breakage; keep a tolerant parser and a clear "unsupported response" state. Many tracker issues Sep-Nov 2026.
- Auth friction: encrypted `state.vscdb` token (DPAPI-backed on Windows) -> rely on CLI `auth.json` or user login via `agent login`; token expiry with no safe refresh; stale IDE token vs fresh CLI token.
- Different token forms per host (bare JWT Bearer on api2.cursor.sh vs `userId::jwt` cookie on cursor.com).
- CSRF/Origin check on POST dashboard endpoints; 403 on the events endpoint after heavy paging (cooldown). `cursor.com` may return an HTML page with HTTP 200 (seen by usage-monitor). No explicit anti-bot (Cloudflare challenge) documentation found for api2.cursor.sh [?].
- Team/enterprise seats return different or empty shapes.
- ToS: not checked (no ToS fetched). Using the user's own token against private endpoints is a gray area; CodexBar/OpenUsage etc. do it. Read-only, local credentials, no sharing is low practical risk but not endorsed by Cursor [?]. Never log/store tokens; open the DB read-only.
