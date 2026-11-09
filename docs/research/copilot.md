# GitHub Copilot: usage tracking research (2026-11-09)

Method: web sources only (no local files or credentials touched). [?] = uncertain, [3p] = only seen in third-party issues/PRs/docs, not in GitHub docs.

## TL;DR
- Limits: YES, via the undocumented `GET https://api.github.com/copilot_internal/user` (the endpoint VS Code and the Copilot CLI `/usage` use). An official billing REST API exists but reports spend/credits per username, not "% of limit".
- Tokens: PARTLY. Copilot CLI writes per-session, per-model token totals to `~/.copilot/session-state/<id>/events.jsonl`. VS Code Copilot Chat stores per-request prompt/output tokens in its chat session files (builds from ~Apr 2026), no cache split.
- Hub: NO. CLIProxyAPI core has no Copilot provider.

## 0. Billing context (affects display)
- Since 2026-06-01 all plans moved from premium request units to usage-based "GitHub AI Credits" (1 credit = $0.01, token based). Annual Pro/Pro+ subscribers stay on premium requests until expiry. Completions / next-edit suggestions use no credits. https://github.blog/news-insights/company-news/github-copilot-is-moving-to-usage-based-billing/ , https://docs.github.com/en/copilot/concepts/billing-and-usage/individuals/billing
- Monthly allowance (credits, base+flex): Pro 1,500, Pro+ 7,000, Max 20,000 (docs billing page). Business $19 / Enterprise $39 per user, pooled across the org (blog). Free: some credits, amount not stated.
- Reset: 1st of each calendar month 00:00 UTC, independent of billing date (docs billing page).
- So `copilot_internal/user` has two generations: legacy premium-request counts (entitlement/remaining) and newer `token_based_billing` / `credits_used` fields [3p]. The parser must tolerate both.

## 1. Credentials (Windows)
- Copilot CLI: OAuth token (`gho_`) in Windows Credential Manager, service name `copilot-cli`. Plaintext fallback `~/.copilot/config.json` (or `$COPILOT_HOME`) only when no keychain. Precedence: `COPILOT_GITHUB_TOKEN` > `GH_TOKEN` > `GITHUB_TOKEN` > keychain OAuth > `gh auth token`. Supported: `gho_`, fine-grained PAT with "Copilot Requests", `ghu_`; classic `ghp_` NOT supported. https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/authenticate-copilot-cli
  - Exact Credential Manager target/account string is not documented [?]; enumerate by the `copilot-cli` prefix at implementation time.
- gh CLI: `gh auth token [-h host] [-u user]` prints the token (may refresh). Storage is keyring or `hosts.yml` (`%APPDATA%\GitHub CLI\hosts.yml`) [?: not stated on https://cli.github.com/manual/gh_auth_token]. A gh token was reported to work against `copilot_internal/user` [3p] https://github.com/BasantPandey/AIQuotaTool/issues/96
- VS Code: token is held by VS Code's built-in GitHub auth provider (scopes `read:user`, also sets with `user:email`, `repo`, `workflow`): https://raw.githubusercontent.com/microsoft/vscode/main/product.json . It sits in encrypted SecretStorage [?, not verified]; not practically readable by a third party. Use the CLI/gh token instead.
- Which token works: a user OAuth token from an OAuth app GitHub recognises (VS Code session, Copilot CLI `gho_`, gh token, classic PAT with `copilot` scope per one report). A token minted by an unrelated extension's own OAuth app (client Ov23liNRlhzedfjImsrQ) was REJECTED, scope was not the cause [3p] (AIQuotaTool #96 above).
- Login/email: response of `copilot_internal/user` carries `login` [3p] https://github.com/kunchenguid/quota-axi/pull/200 ; otherwise `GET https://api.github.com/user` (`login`; `email` needs `user:email`).

## 2. Quota endpoint (undocumented)
- `GET https://api.github.com/copilot_internal/user`; listed as `entitlementUrl` in VS Code product.json (primary). Enterprise data residency: `api.<tenant>.ghe.com` [3p].
- Auth: `Authorization: token <gho_...>` (CodexBar docs) or `Authorization: Bearer <token>` (VS Code/quota-axi reports); both seen. Do NOT use the exchanged Copilot JWT from `/copilot_internal/v2/token`.
- Headers one open-source tracker sends (https://github.com/steipete/CodexBar/blob/main/docs/copilot.md): `Accept: application/json`, `Editor-Version: vscode/1.96.2`, `Editor-Plugin-Version: copilot-chat/0.26.7`, `User-Agent: GitHubCopilotChat/0.26.7`, `X-Github-Api-Version: 2025-04-01`. Whether the editor headers are required is unknown [?]; another tracker sends only Authorization.
- Response (made-up values; field names from https://github.com/getpaseo/paseo/issues/8478 , https://github.com/pingdotgg/t3code/issues/22862 , https://github.com/kunchenguid/quota-axi/pull/200 , AIQuotaTool #96):
```json
{
  "login": "octocat",
  "copilot_plan": "individual",
  "access_type_sku": "free_limited_copilot",
  "quota_reset_date": "2026-12-01",
  "quota_reset_date_utc": "2026-12-01T00:00:00.000Z",
  "token_based_billing": true,
  "quota_snapshots": {
    "premium_interactions": {
      "quota_id": "premium_interactions",
      "entitlement": 1500, "remaining": 1230, "percent_remaining": 82.0,
      "unlimited": false, "overage_permitted": true, "overage_count": 0,
      "credits_used": 270, "quota_reset_at": 0
    },
    "chat":        { "entitlement": 200,  "remaining": 196,  "percent_remaining": 98.3, "unlimited": false, "overage_permitted": false },
    "completions": { "entitlement": 2000, "remaining": 2000, "percent_remaining": 100.0, "unlimited": true }
  }
}
```
- Quirks: `entitlement` was a number, later seen as a string (accept both) [3p]; `quota_reset_at` is Unix seconds and often `0` -> fall back to top-level `quota_reset_date_utc` (else 1970) [3p]; skip `unlimited:true` and `entitlement:0` snapshots (or show "Unlimited"); `access_type_sku:"subscription_ended"` = no active sub (CodexBar docs). Semantics/units of `token_based_billing` and `credits_used` are undocumented [?]; CodexBar notes premium `credits_used` is the seat AI-credit total and must not be summed with chat/completions credits.
- Poll gently: one tracker advises at most once per 5 min [3p].
- Official alternatives:
  - Copilot SDK `account.getQuota` (`quotaSnapshots`: `entitlementRequests` (-1 = unlimited), `usedRequests`, `remainingPercentage`, `resetDate`); needs the CLI/SDK runtime. https://docs.github.com/en/copilot/how-tos/copilot-sdk/features/usage-and-billing
  - Billing REST (`X-GitHub-Api-Version: 2026-03-10`): `GET /users/{username}/settings/billing/ai_credit/usage` and `/premium_request/usage` (and `/organizations/{org}/settings/billing/...`, org admin only); params year/month/day/model/product; items have product, sku, model, unitType, pricePerUnit, grossQuantity, netQuantity, netAmount. Covers only usage billed to the personal account; gives spend, not the allowance, so no % without hardcoding plan credits. Required token permission not stated. https://docs.github.com/en/rest/billing/usage

## 3. Mapping to "% used + reset"
- Primary window: `premium_interactions` -> used% = 100 - percent_remaining; reset = snapshot `quota_reset_at` if > 0 else `quota_reset_date_utc` (1st of month 00:00 UTC per docs).
- Secondary windows (mostly Free tier): `chat`, `completions` monthly counts; skip if unlimited.
- Show plan from `copilot_plan` / `access_type_sku`; show overage when `overage_count > 0`.
- Business/Enterprise: endpoint returns the seat's snapshot; whether the pooled org allowance is reflected is unknown [?].

## 4. CLIProxyAPI
- No Copilot support in core. README providers: Kimi, OpenAI/Codex, Claude, Gemini/Antigravity, xAI, Meta, Devin. `internal/auth` dirs: antigravity, claude, codex, devin, empty, kimi, meta, vertex, xai. Management login endpoints: anthropic/codex/antigravity (browser), kimi/xai (device); none for Copilot. `GET /auth-files` entries: `provider`, `email`, `account_type`, `status`... https://github.com/router-for-me/CLIProxyAPI , https://help.router-for.me/management/api (page says /v0/management is near deprecation; v8 exists).
- Wrappers (ProxyPal, Quotio) mention Copilot, probably via forks [?]. Even then the hub would not expose `copilot_internal/user` quotas.

## 5. Local logs with tokens
- Copilot CLI: `${COPILOT_HOME:-%USERPROFILE%\.copilot}\session-state\<session-uuid>\events.jsonl` (JSONL).
  - `session.start`: `data.producer:"copilot-agent"`, `data.copilotVersion`, `data.context.cwd/gitRoot`.
  - `session.shutdown` (cumulative per session): `data.modelMetrics.<model>.usage.{inputTokens,outputTokens,cacheReadTokens,cacheWriteTokens,reasoningTokens}`, `...requests.{count,cost}`, `data.totalNanoAiu` (credits; 1 AIU = 1 credit = $0.01), `data.tokenDetails.cache_read.tokenCount`. `inputTokens` INCLUDES cache buckets; `reasoningTokens` is already inside `outputTokens`. Running/crashed sessions have no shutdown event -> undercount.
  - `session.usage_checkpoint` (credit total only) and, in newer CLI (~1.0.105+), `session.usage_record` per model call (tokens, model, cost, duration) [3p].
  - Live-only `assistant.usage` (model, inputTokens, outputTokens, cost) is not replayed from disk.
  - Example (made-up): `{"type":"session.shutdown","data":{"shutdownType":"routine","modelMetrics":{"claude-sonnet-5":{"usage":{"inputTokens":120000,"outputTokens":3400,"cacheReadTokens":90000,"cacheWriteTokens":8000,"reasoningTokens":400}}},"totalNanoAiu":42000000000}}`
  - Opt-in OTel file export: `COPILOT_OTEL_ENABLED=true`, `COPILOT_OTEL_EXPORTER_TYPE=file`, `COPILOT_OTEL_FILE_EXPORTER_PATH=...`; `~/.copilot/otel/**/*.jsonl` with `gen_ai.usage.input_tokens` etc.
  - Sources: https://ccusage.com/guide/copilot/ , https://github.com/getagentseal/codeburn/issues/944 , https://github.com/ccusage/ccusage/issues/1174 , SDK doc above.
- VS Code Copilot Chat: `%APPDATA%\Code\User\workspaceStorage\<hash>\chatSessions\*.jsonl` (mutation log, kinds 0 init/1 set/2 push/3 delete, must be replayed; older builds whole-file JSON), `globalStorage\emptyWindowChatSessions\*.jsonl`, and `workspaceStorage\<hash>\GitHub.copilot-chat\transcripts\<sessionId>.jsonl` (event log; `outputTokens` is 0, no shutdown rollup). Per request after replay: `requests[].result.metadata.promptTokens` / `outputTokens` (builds since ~Apr 2026; some entries lack them; no cache split; where the model name lives not confirmed [?]). Optional debug logs `GitHub.copilot-chat/debug-logs/` (per round tokens + credits) need a setting. Insiders/profile paths differ. Sources (all 3p, internal unstable format): https://vshulcz.github.io/deja-vu/registry/copilot-chat.html , https://github.com/junhoyeo/tokscale/issues/1551 , codeburn #944.
- JetBrains / other IDEs: not researched.

## 6. Own device-flow login
- Standard RFC 8628: `POST https://github.com/login/device/code` (`client_id`, `scope=read:user`), user enters code at https://github.com/login/device, poll `POST https://github.com/login/oauth/access_token` with `grant_type=urn:ietf:params:oauth:grant-type:device_code` -> `gho_` token. Enterprise: same paths on the tenant host. CLI flow doc: authenticate-copilot-cli page above.
- Client id widely reused by third parties for the Copilot editor app: `Iv1.b507a08c87ecfe98` (https://docs.litellm.ai/docs/providers/github_copilot). Public (not secret) but owned by GitHub's Copilot editor integration, not by us. The Copilot CLI's own OAuth app id was not found [?].
- ToS risk: posing as GitHub's editor client (plus editor-version headers) to call `copilot_internal/*` is unsanctioned; no explicit ToS clause naming it was found [?], but GitHub can block or restrict it. Lower-risk order: (1) read-only reuse of credentials the user already has (CLI keychain / gh), (2) user-pasted fine-grained PAT (note: PAT support for `copilot_internal/user` unverified [?]), (3) own device flow with a SubHub-registered OAuth app (must verify the endpoint accepts its tokens; a third-party app's tokens were rejected in one report).

## 7. Risks
- `copilot_internal/*` is undocumented; shape has already shifted (token-based billing, entitlement type) and may again. Treat parse failure as "no data"; never invent 100%.
- Only tokens from recognised apps work; `GH_TOKEN`/`GITHUB_TOKEN` env vars may hold unrelated tokens.
- Reading another app's Credential Manager entry (`copilot-cli`) may trip security tooling; read-only, never log tokens.
- Rate limits of this endpoint unknown [?]; api.github.com general limits apply; poll >= 5 min.
- Quotas vary by plan/contract (annual plans still on premium requests; Business/Enterprise pooled).
- Local logs: undocumented formats that change across CLI versions (event types shifted ~1.0.74-1.0.105); VS Code logs need replay; totals undercount live sessions.
