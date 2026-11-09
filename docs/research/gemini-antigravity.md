# Research: Gemini CLI and Google Antigravity for AI SubHub

Researched 2026-11-09 from web sources only (no local credential files touched). "(?)" marks uncertain points. All endpoints are internal/undocumented unless stated.

## 0. Headline finding: the consumer Gemini CLI login is dead

Google stopped serving Gemini CLI (and Code Assist IDE extensions) on 2026-06-18 for Code Assist for individuals, Google AI Pro and Google AI Ultra. "Login with Google" no longer works for those tiers; Google points consumers to Antigravity / Antigravity CLI (`agy`). Standard/Enterprise (Workspace) tiers are "subject to a separate deprecation schedule" (so they also sunset eventually). API-key and Vertex auth are not mentioned by Google.
- https://developers.google.com/gemini-code-assist/docs/deprecations/code-assist-individuals
- Confirmed in CodexBar docs/gemini.md ("Consumer-tier migration (June 2026)"): https://github.com/steipete/CodexBar/blob/main/docs/gemini.md
- CLIProxyAPI users report Gemini CLI OAuth failing since ~June 2026: https://github.com/router-for-me/CLIProxyAPI/issues/3915

Consequence: Gemini CLI quota tracking only still makes sense for Workspace/Code Assist Standard/Enterprise accounts. Free/Pro/Ultra consumers are on Antigravity now (shared Google AI Pro/Ultra subscription).

---

## 1. Gemini CLI

### 1.1 Local credentials (Windows)
- Legacy file: `%USERPROFILE%\.gemini\oauth_creds.json` (`Storage.getOAuthCredsPath` = `<home>/.gemini/oauth_creds.json`). Fields (google-auth-library `Credentials`): `access_token`, `refresh_token`, `scope`, `token_type`, `id_token`, `expiry_date` (ms epoch). Source: https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/config/storage.ts and https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/code_assist/oauth2.ts
- Newer CLI versions store credentials via `OAuthCredentialStorage` in a **keychain** (service `gemini-cli-oauth`, account `main-account`; Windows Credential Manager via keytar-like service, encrypted-file fallback), and migrate/delete the old `oauth_creds.json` on first load. Source: https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/code_assist/oauth-credential-storage.ts
  (?) `cacheCredentials()` in oauth2.ts still writes the JSON file, so which one exists on a given machine depends on the CLI version. Treat the file as "maybe present", keychain as the modern location. Keychain blob layout (`token.accessToken/refreshToken/expiresAt/tokenType/scope`) is the MCP token-storage format, not verified byte-for-byte.
- Account email: `%USERPROFILE%\.gemini\google_accounts.json` → `{"active": "<email>", "old": ["..."]}` (cached after login via `GET https://www.googleapis.com/oauth2/v2/userinfo`). Also the `email` claim of the `id_token` JWT (CodexBar uses this). Sources: userAccountManager.ts and oauth2.ts (`fetchAndCacheUserInfo`) in the same folder.
- Refresh: `POST https://oauth2.googleapis.com/token`, form body `client_id`, `client_secret`, `refresh_token`, `grant_type=refresh_token`. Client ID is public in gemini-cli source (`oauth2.ts`: `681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com`), with an "installed app" client secret also hard-coded in that file (Google's own comment says it is not treated as secret). Read it from the CLI source/package at implementation time; do not copy into docs. Scopes: `cloud-platform`, `userinfo.email`, `userinfo.profile`.
- Env override `GEMINI_CLI_HOME`/home dir handling: `homedir()` in utils/paths.ts (?).

### 1.2 Quota / usage endpoint
Base: `https://cloudcode-pa.googleapis.com/v1internal` (override env `CODE_ASSIST_ENDPOINT`). Method URL = `<base>:<method>`. Source: server.ts (`getMethodUrl`), https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/code_assist/server.ts

1. Project/tier discovery: `POST /v1internal:loadCodeAssist`
   - Headers: `Authorization: Bearer <access_token>`, `Content-Type: application/json`
   - Body: `{"metadata":{"ideType":"GEMINI_CLI","pluginType":"GEMINI"}}` (CodexBar form; CLI sends extra platform fields). Optional `cloudaicompanionProject`, `mode`: `HEALTH_CHECK`.
   - Response (types.ts `LoadCodeAssistResponse`): `currentTier{id,name,description,...}`, `allowedTiers[]`, `ineligibleTiers[{reasonCode,reasonMessage,tierId,tierName}]`, `cloudaicompanionProject` (string), `paidTier{id,name,availableCredits[]}`.
   - Example (made up): `{"currentTier":{"id":"standard-tier","name":"Gemini Code Assist Standard"},"cloudaicompanionProject":"my-proj-123","paidTier":{"id":"g1-pro-tier","name":"Google AI Pro"}}`
   - Tier ids seen: `free-tier`, `standard-tier`, `legacy-tier`; CodexBar plan labels: `paidTier.name` wins, else `standard-tier` -> Paid, `free-tier`+`hd` claim -> Workspace, `free-tier` -> Free. https://github.com/steipete/CodexBar/blob/main/docs/gemini.md
   - If `currentTier` is absent and `ineligibleTiers[].reasonCode == "UNSUPPORTED_CLIENT"` -> consumer account shut down (see section 0). `retrieveUserQuota` then returns 403 `SUBSCRIPTION_REQUIRED`.
   - Fallback project lookup: `GET https://cloudresourcemanager.googleapis.com/v1/projects` and pick `gen-lang-client*` (CodexBar).
2. Quota: `POST /v1internal:retrieveUserQuota`
   - Body: `{"project":"<cloudaicompanionProject>"}` (`{}` works if unknown, per CodexBar).
   - Response (types.ts `RetrieveUserQuotaResponse`/`BucketInfo`): `buckets[]` with `modelId`, `remainingFraction` (0..1), `remainingAmount` (string), `resetTime` (ISO-8601), `tokenType` (e.g. `REQUESTS`).
   - Example (made up):
     ```json
     {"buckets":[
       {"modelId":"gemini-2.5-pro","tokenType":"REQUESTS","remainingAmount":"912","remainingFraction":0.912,"resetTime":"2026-11-10T07:12:00Z"},
       {"modelId":"gemini-2.5-flash","tokenType":"REQUESTS","remainingAmount":"990","remainingFraction":0.99,"resetTime":"2026-11-10T07:12:00Z"}]}
     ```
   - Used% = `(1 - remainingFraction) * 100`.

### 1.3 Windows / mapping
Windows are per model, effectively a daily request quota (docs: "maximum requests per user per day": 1000 individual / 1500 Pro / 2000 Ultra / 1500 Standard / 2000 Enterprise, plus a per-minute limit) - https://github.com/google-gemini/gemini-cli/blob/main/docs/resources/quota-and-pricing.md (repo doc may be stale for consumer tiers). Each bucket has its own `resetTime`. Display idea (CodexBar does the same): group buckets by family (Pro / Flash / Flash-Lite), take the lowest `remainingFraction` per family, show used% + reset time. No separate weekly window.

### 1.4 CLIProxyAPI
- Provider/type value is `gemini-cli` (file `"type":"gemini"` is normalized to `gemini-cli`; metadata has `project_id`). Source: https://github.com/router-for-me/CLIProxyAPI/blob/main/internal/watcher/synthesizer/file.go
- Hub still *loads* existing `gemini-cli` auth files, but current main (v8) has **no login URL route** for it: routes are only `anthropic|codex|antigravity|kimi|kimi-ai|xai|devin|meta-auth-url` (v0) and `/v8/management/oauth/auth-url?provider=<claude|codex|antigravity|kimi|kimi-ai|xai|devin|meta|plugin>`. Sources: https://github.com/router-for-me/CLIProxyAPI/blob/main/internal/api/server_management.go and https://github.com/router-for-me/CLIProxyAPI/blob/main/docs/management-api-v8.md. Older docs mention `gemini-cli-auth-url` (with project_id) - not in current source (?). Gemini CLI login with the hub is also moot after June 2026 for consumers.
- `api-call` token substitution: `$TOKEN$` resolves `metadata.access_token`, then `attributes.api_key`, then `metadata.token/id_token/cookie`; explicit refresh logic exists only for `antigravity`, `meta`, `xai` providers, **not** for `gemini-cli` (api_tools.go `resolveTokenForAuth`). So an expired gemini-cli token is not refreshed by api-call (?) - and issue #5489 reports expired-access-token gemini-cli creds being excluded. https://github.com/router-for-me/CLIProxyAPI/blob/main/internal/api/handlers/management/api_tools.go , https://github.com/router-for-me/CLIProxyAPI/issues/5489
  Reaching `retrieveUserQuota` via `api-call` should work in principle (arbitrary absolute URL + `Authorization: Bearer $TOKEN$`), with `project_id` from the `auth-files` entry (which exposes `project_id`, `email`, `auth_index`, `type`, `provider`). Not verified live.

### 1.5 Local session logs / tokens
- Path: `%USERPROFILE%\.gemini\tmp\<project-id>\chats\session-<timestamp>-<shortid>.jsonl` (subagent sessions in a nested folder). `<project-id>` is a short id from `~/.gemini/projects.json` registry (older versions: SHA-256 hash of project root, `projectHash`). Sources: storage.ts (`getProjectTempDir`), chatRecordingService.ts (filename build), projectRegistry.ts, all under https://github.com/google-gemini/gemini-cli/tree/main/packages/core/src
- Format: JSONL, first line = metadata `{sessionId, projectHash, startTime, lastUpdated, kind}`, then one message record per line; also patch lines (`{"$patch":...}`), `{"$set":...}`, `{"$rewindTo":...}` (must be applied/skipped; rewinds drop messages). Legacy versions wrote one `.json` `ConversationRecord`. Types: https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/services/chatRecordingTypes.ts
- Token fields on `type:"gemini"` messages: `model`, `tokens{input,output,cached,thoughts,tool,total}` (input=promptTokenCount, output=candidatesTokenCount, cached=cachedContentTokenCount).
- Example line (made up):
  `{"id":"m42","timestamp":"2026-11-09T10:15:03.120Z","type":"gemini","content":"...","model":"gemini-2.5-pro","tokens":{"input":18234,"output":512,"cached":15000,"thoughts":220,"tool":0,"total":18966}}`
  Note: tokens may be attached to the last gemini message after the fact (`recordMessageTokens` sets `lastMsg.tokens` or queues them) - so dedupe by message id.

### 1.6 Risks
- Consumer login discontinued (above). Endpoints `cloudcode-pa.googleapis.com/v1internal` are internal/undocumented and can change anytime. Using the CLI's own OAuth client from a third-party app is a grey area; CLIProxyAPI users had Gemini CLI accounts suspended (https://github.com/router-for-me/CLIProxyAPI/issues/1803). Credentials moving to keychain means the JSON file may not exist. Workspace tiers face their own sunset.

---

## 2. Google Antigravity

Two products: the desktop IDE/app and the **Antigravity CLI (`agy`)**, successor to Gemini CLI. Plans: free (weekly quota only), Google AI Pro and Ultra (5-hour quota capped by a weekly limit). Third-party models (Claude/GPT) in a separate pool; Ultra benefit and Pro non-trial per CLIProxyAPI README. https://antigravity.google/docs/plans

### 2.1 Local credentials (Windows)
- `agy` token: **Windows Credential Manager**, generic credential target `gemini:antigravity`, user `antigravity` (agy 1.4.3+). UTF-8 JSON blob: `token{access_token,token_type,refresh_token,expiry,expires_in}`, `auth_method` (e.g. `"consumer"`), `id_token`. Read with `CredReadW`. https://github.com/s3ntin3l8/runway-ai-usage-tracker/issues/728
- File fallback (keyring unavailable / older agy): `%USERPROFILE%\.gemini\antigravity-cli\antigravity-oauth-token` (JSON, same shape (?)). https://github.com/agentistics/agentistics/issues/1221 (search snippet; page not fully read)
- Install dir `%LOCALAPPDATA%\agy\bin` (agy 1.3.5 observation, from a search snippet).
- Desktop app / IDE: credentials location on Windows not established (?). CodexBar does not read them; it queries the app's local language server instead.
- Refresh: `POST https://oauth2.googleapis.com/token`, form `client_id`, `client_secret`, `refresh_token`, `grant_type=refresh_token`. Client for Antigravity login: ID `1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com` (public constant in CLIProxyAPI `internal/auth/antigravity/constants.go`; secret is in the same file). Caveat: refresh tokens are bound to the client that issued them; CodexBar notes Antigravity 2.x ships two clients and a mismatch gives `GOOGLE_TOS_NOT_SUPPORTED_BY_CLIENT` / `invalid_client` (?: which client agy uses today). https://github.com/steipete/CodexBar/blob/main/docs/antigravity.md
- Email: `id_token` `email` claim, or `GET https://www.googleapis.com/oauth2/v2/userinfo?alt=json`.
- OAuth scopes (CLIProxyAPI): cloud-platform, userinfo.email, userinfo.profile, aicode, cclog, experimentsandconfigs. Local callback port 51121.

### 2.2 Quota endpoints (Cloud Code, same family as Gemini CLI)
All `POST`, JSON, `Authorization: Bearer <access_token>`, `Content-Type: application/json`. CodexBar sends `User-Agent: antigravity/hub/2.9.1 <os>/<arch>` and notes "Cloud Code requires the Hub client family for quota access" (a UA tied to the installed agy version matters: with UA `1.4.3` three pools return, an old pinned `1.0.9` only one - runway issue #728). Sources: https://github.com/steipete/CodexBar/blob/main/Sources/CodexBarCore/Providers/Antigravity/AntigravityRemoteUsageFetcher.swift and docs/antigravity.md.

1. `POST https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist`, body `{"metadata":{"ideType":"ANTIGRAVITY","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}}`. Response fields used: `cloudaicompanionProject` (string or `{id|projectId}`), `currentTier{id,name}`, `paidTier{id,name}`, `allowedTiers[{id,isDefault}]`, `ineligibleTiers[{reasonCode}]`, `planInfo.planType`. If no project: `POST ...:onboardUser` with `{"tierId":"<default allowed tier>","metadata":{...}}` -> `response.cloudaicompanionProject`.
2. Preferred, grouped 5h + weekly: `POST https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary`, body `{"project":"<id>"}`. Response (shape from CodexBar's parser; may be wrapped in `response`/`summary`):
   ```json
   {"groups":[
     {"displayName":"Gemini Models","buckets":[
       {"bucketId":"gemini-5h","displayName":"5-hour limit","window":"5h","remaining":{"remainingFraction":0.8},"resetTime":"2026-11-09T15:00:00Z","description":"Resets in 3h","disabled":false},
       {"bucketId":"gemini-weekly","displayName":"Weekly limit","remaining":{"remainingFraction":0.95},"resetTime":"2026-11-14T00:00:00Z"}]},
     {"displayName":"Claude and GPT models","buckets":[{"bucketId":"...","remaining":{"remainingFraction":1.0}}]}]}
   ```
   `remainingFraction` may be top-level or under `remaining`. Starter (free) accounts only get weekly buckets. Hub-request UA required (see above).
3. Fallbacks (per-model): `POST https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels` body `{"project":"<id>"}` -> `{"models":{"<model-id>":{"displayName","quotaInfo":{"remainingFraction","resetTime"}}}}`; and `...:retrieveUserQuota` -> same `buckets[]` shape as Gemini (section 1.2). CodexBar treats an all-100% `fetchAvailableModels` as "availability only, not measured" unless `retrieveUserQuota` confirms.
4. Local alternative: the running app / `agy` exposes a Connect-RPC server on localhost (`/exa.language_server_pb.LanguageServerService/RetrieveUserQuotaSummary`, `GetUserStatus`), but `agy` >= 1.2.2 requires a CSRF token that third parties cannot get; the supported non-interactive way is `agy -p /usage --output-format json` (agy >= 1.1.11), giving `{"status":"SUCCESS","command":{"name":"usage","data":{groups:[...]}}}` without account/plan identity. Set `AGY_CLI_DISABLE_AUTO_UPDATE=true` for probes. Recommended for SubHub: spawn `agy -p /usage --output-format json` (no token handling) as the primary or fallback path. Source: docs/antigravity.md above.

### 2.3 Windows
Two groups, each with `5h` and `weekly` buckets: "Gemini Models" and "Claude and GPT models". Maps 1:1 to SubHub's "5-hour + weekly, % used + reset time" (used = 1 - remainingFraction). Free plan: weekly only. Plan label: `paidTier.name` preferred over `currentTier`.

### 2.4 CLIProxyAPI
- `type`/`provider` = `antigravity`. Login: `GET /v0/management/antigravity-auth-url` -> `{"status":"ok","url":"<google auth url>","state":"<state>"}`; poll `GET /v0/management/get-auth-status?state=<state>` (`wait` / `ok` / `error`); v8 equivalent `GET /v8/management/oauth/auth-url?provider=antigravity`. No params needed; project_id is auto-discovered after login and stored in metadata (`project_id`). OAuth callback port 51121. Sources: server_management.go, auth_files_provider_oauth.go in https://github.com/router-for-me/CLIProxyAPI/tree/main/internal/api/handlers/management
- `auth-files` entries include `auth_index`, `type`, `provider`, `email`, `project_id`, `status`, `account`.
- `api-call` **refreshes the Antigravity access token itself** before substituting `$TOKEN$` (uses the Antigravity client, form refresh against oauth2.googleapis.com), so `POST /v0/management/api-call` with `{"auth_index":"..","method":"POST","url":"https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels","header":{"Authorization":"Bearer $TOKEN$","Content-Type":"application/json","User-Agent":"<antigravity UA>"},"data":"{\"project\":\"<project_id>\"}"}` is the documented pattern (api_tools.go example uses exactly fetchAvailableModels). `retrieveUserQuotaSummary` through the same mechanism is expected to work but is unverified (?); send the Hub-style User-Agent. Response wrapper: `{status_code, header, body(string)}`.
- v0 stays supported alongside v8 (docs/management-api-v8.md).

### 2.5 Local session logs / tokens
Yes, but not simple. Conversations: SQLite DBs `%USERPROFILE%\.gemini\antigravity-cli\conversations\*.db`, `%USERPROFILE%\.gemini\antigravity\*.db`, `%USERPROFILE%\.gemini\antigravity\conversations\*.db` (`GEMINI_CLI_HOME` replaces `~/.gemini`). Table `gen_metadata(idx, data)`; `data` is a protobuf blob. Usage fields (per CodexBar/Tokscale/ccusage reverse engineering): field 1 = model enum id, 2 = input tokens, 5 = cache read, 9 = reasoning output, 10 = text output; timestamp at `chatModel.#9.#4` (seconds/nanos). Model id -> name mapping is by enum and needs a table (e.g. 1298 = `gemini-3.7-flash`). `conversation_summaries.db` is not history. Alternative JSONL cache from Tokscale: `~/.config/tokscale/antigravity-cache/sessions/*.jsonl` (fields `sessionId`, `output`, `reasoning`). Schema is undocumented and drifts per agy version (1.1.18 timestamp layout unsupported). Sources: https://github.com/steipete/CodexBar/blob/main/docs/antigravity.md (cites Tokscale `antigravity_cli.rs` and ccusage antigravity adapter). Verdict: feasible but fragile; do last. No plain-text session log with token counts.

### 2.6 Risks
- Google's Antigravity Additional Terms clause 6 (as quoted by third parties): "Using third party software, tools, or services to access the Service (e.g. using OpenClaw with Antigravity OAuth) is a breach of this Agreement." Accounts (including paid Ultra) were suspended from ~2026-02-09 with 403 `TOS_VIOLATION`, appeals rarely succeed; CLIProxyAPI users reported bans (issues #1803, #1637). Sources: https://zentor.ai/blog/antigravity-account-suspended-third-party , https://github.com/router-for-me/CLIProxyAPI/issues/1803 , https://discuss.ai.google.dev/t/request-for-manual-review-antigravity-access-disabled-403-tos-violation-third-party-integration-removed/198604 . These bans concern *inference* through third-party tools; read-only quota polling is lower risk but also unofficial (not established either way). Using the Antigravity OAuth client from SubHub = same grey area. Safer: `agy -p /usage` (official binary, no token handling) or the hub.
- Credential storage moved (file -> Credential Manager at agy 1.4.3) and may move again; `User-Agent` gating on quota endpoints; two OAuth clients; undocumented protobuf for logs; `agy` print-mode may launch a browser if sign-in fails.

---

## 3. Verdicts

| | Limits | Tokens | Hub |
|---|---|---|---|
| Gemini CLI | Yes, but only Workspace/Standard/Enterprise (consumer shut down 2026-06-18) | Yes (JSONL in `~/.gemini/tmp/<id>/chats`, clean fields) | Weak: existing `gemini-cli` files load, no login route in current CLIProxyAPI, no token refresh in api-call |
| Antigravity | Yes (5h + weekly, Gemini and Claude/GPT groups) | Partial (SQLite + protobuf, fragile) | Yes (`type: antigravity`, login URL, api-call refreshes token) |

## 4. Things not verified (need live check before building)
- Exact request/response of `retrieveUserQuotaSummary` against a real account and whether it works via hub `api-call`.
- Which OAuth client current agy/the hub tokens are bound to (refresh may fail with `invalid_client`).
- Whether Gemini CLI on a Workspace account on Windows keeps `oauth_creds.json` or only the keychain.
- Antigravity desktop credential location on Windows.
