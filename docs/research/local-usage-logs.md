# Local usage logs: Copilot CLI, Cursor, Antigravity (agy)

Researched 2026-11-09, web sources only. On this PC only directory existence was checked (no files opened, nothing installed). All formats below are undocumented internals unless stated, so expect drift. "[3p]" = only seen in third-party docs/issues. "[?]" = unverified.

What SubHub's scanner needs per request: timestamp, model, input / cache read / cache write / output tokens, ideally cwd and a session id (see `crates/subhub-core/src/stats/scan.rs`: hourly buckets per model, incremental by byte offset over JSONL).

## Verdicts

| Provider | Local per-request tokens + model? | Verdict | What SubHub could show |
|---|---|---|---|
| Copilot CLI | Per session and model, at shutdown (cumulative JSONL snapshots). Per call only live, not on disk. | Partially feasible | Tokens per day/model (day = shutdown time, not call time), cwd from `session.start`; cost from token prices or from the logged AI credits. Live/crashed sessions undercount. |
| Copilot in VS Code | Per turn, in chat session mutation logs [3p] | Partially feasible, later | Same, needs a replay parser; no cache write split. Out of scope for the CLI-only step. |
| Cursor (CLI and IDE) | CLI: no. IDE: `tokenCount` only in older chats; reportedly dropped early 2026 [3p]. Real numbers are server-side only. | Not feasible as a local log scan | Only via the (undocumented, authenticated) usage-events API, i.e. a network feature, not a log parser. |
| Antigravity `agy` | Yes, in per-conversation SQLite, protobuf blobs [3p] | Feasible but fragile | Tokens per day/model (input, cache read, output, reasoning) from `gen_metadata`; needs a protobuf reader and a model-enum table; no cwd confirmed. |

## 1. GitHub Copilot CLI

**Location (Windows).** `%USERPROFILE%\.copilot\session-state\<session-id>\events.jsonl`; `COPILOT_HOME` replaces the `.copilot` root (ccusage guide, kibble post; not in official docs, `--config-dir` also mentioned [3p]). Official docs confirm the `~/.copilot/session-state/` folder holds the full session record and that a SQLite "session store" holds a subset (it can be rebuilt with `/chronicle reindex`). Older layouts: single `{uuid}.jsonl` files, pre-0.0.342 `history-session-state/` [3p]. On this PC `%USERPROFILE%\.copilot` exists, `session-state` and `otel` do not.

**Format.** JSONL, one object per line: `type`, `timestamp` (ISO 8601 UTC), `id`, `data`. Next to it `workspace.yaml` [3p].

**Token fields.**
- `session.shutdown` (persisted): `data.modelMetrics.<model>.usage.{inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens, reasoningTokens}`, `...requests.{count, cost}`, `data.totalNanoAiu`, `data.sessionStartTime`, `data.currentModel`. The official SDK docs document `modelMetrics` as `Record<string, ModelMetric>`; the inner field names are [3p].
- Semantics [3p, consistent across ccusage, kibble, codeburn]: `inputTokens` includes both cache buckets (uncached = input - read - write); `reasoningTokens` is a subset of output; snapshots are cumulative per session and model, a resumed session appends another shutdown with a higher total, so take differences between consecutive snapshots (or the latest per session/model); `modelMetrics` can be empty; sub-agent records (`agentId`) are already inside the parent's totals.
- `session.usage_checkpoint` (persisted): only `totalNanoAiu` (credits, 1 AIU = 1 credit = $0.01) and optional `totalPremiumRequests`.
- `assistant.usage` (per API call: model, input/output/cache tokens, cost) is documented as ephemeral: streamed live, not saved, not replayed on resume. A newer per-call `session.usage_record` is mentioned in one report [3p, not in the official event page].
- `session.start.data`: `producer`, `copilotVersion`, `context.{cwd, gitRoot}` [3p, codeburn #944]. Model for a session: keys of `modelMetrics`, plus `session.model_change` events.
- Input tokens, cache read and cache write exist only in `session.shutdown` (codeburn #944); `assistant.message` carries `outputTokens` with a timestamp.

**Consequences for the scanner.** Timestamp granularity is per shutdown (session end), not per request. For hourly buckets SubHub would bucket a session's delta at the shutdown time (a long session lands in one hour), or at best split output tokens by `assistant.message` times. A session that is still running or crashed has no shutdown line, so its usage is missing, not zero. The Codex-style "delta against previous total" logic maps well to differencing shutdown snapshots.

**Other local sources.**
- `~/.copilot/session-store.db` (SQLite, derived index; tokscale reads it as "CLI usage events", schema undocumented, official docs do not list tables or token fields).
- Opt-in OpenTelemetry file export: `COPILOT_OTEL_ENABLED=true`, `COPILOT_OTEL_EXPORTER_TYPE=file`, optional `COPILOT_OTEL_FILE_EXPORTER_PATH`; files under `~/.copilot/otel/**/*.jsonl` with chat spans (`gen_ai.usage.input_tokens` etc.). Per call, but off by default and records no workspace [3p]. Only helps going forward.

**Stability.** Undocumented. An open request (github/copilot-cli #3551, May 2026) asks GitHub to formalize `events.jsonl` with a schema and stability contract; no maintainer answer. Event names already shifted across versions (~1.0.74 to 1.0.105) [3p]. ccusage labels its Copilot support "experimental". A tolerant parser that ignores unknown lines is mandatory.

**Third-party readers (existence only).** ccusage (`session-state/*/events.jsonl` + `otel`), codeburn, tokscale (`session-store.db`, `otel`, `data.db`), clocwork, J-Bax/copilot-token-tracker.

**VS Code Copilot Chat (separate source).** `%APPDATA%\Code\User\workspaceStorage\<hash>\chatSessions\*.jsonl` (first line = initial state, later lines = set/append/delete mutations; older builds one `.json` snapshot; if both exist the `.jsonl` is live), `globalStorage\emptyWindowChatSessions`, `transferredChatSessions`, per-profile paths; Insiders uses `Code - Insiders`. `requests[]` per turn: model in `modelId` or `result.metadata.resolvedModel`; `modelTotals[]` with `inputTokens`, `outputTokens`, `cachedTokens` (input includes cached); older records only `promptTokens`/`completionTokens` of the last call; no cache-write field. Inline completions, next-edit suggestions, cloud agent and code review are not logged locally (kibble post). `workspaceStorage` exists on this PC (contents not read). Needs a mutation-log replay, so it is more work than the CLI.

**Caveats.** CLI sessions are also synced to the GitHub account by default (official docs), but there is no documented API to read tokens back from there. No encryption of the session files was reported. ToS: reading local files that the tool writes for the user is not an access to the service; nothing relevant found.

## 2. Cursor (cursor-agent CLI and IDE)

**CLI (`agent` / `cursor-agent`).** Chats: `~/.cursor/chats/<hash>/<session-uuid>/store.db` (SQLite: `blobs(id, data)`, `meta(key, value)`) plus `meta.json` (`schemaVersion`, `createdAtMs`, `updatedAtMs`, sometimes `cwd`, `title`); with `XDG_CONFIG_HOME` set the root moves to `~/.config/cursor/`; ACP sessions in `acp-sessions/`. Transcripts: `~/.cursor/projects/<project>/agent-transcripts/*.jsonl` (lines with `role` and `message.content` only). `~/.cursor/ai-tracking/ai-code-tracking.db` has code attribution incl. a model name. On Windows the exact roots were not found in sources (the `~` forms are from macOS/Linux checks); on this PC `%USERPROFILE%\.cursor` and `%APPDATA%\Cursor` do not exist. A token-ledger investigation of Cursor Agent 2026.10.01 concluded none of these hold token counts or cost: "does not write token usage to disk"; token field names occur only inside message content. `meta` has `lastUsedModel` and `mode`. Cursor staff confirmed the `chats` location on the forum but it is undocumented and unstable (allaboutcoding article).
- The CLI prints a `usage` block in `--print --output-format json` (camelCase `inputTokens`, `outputTokens`, `cacheReadTokens`, `cacheWriteTokens`; reported for 2026.10.19) [3p, dayamjz/assistant #510/#512]. That is stdout of a run SubHub does not own, not a log.

**IDE.** `%APPDATA%\Cursor\User\globalStorage\state.vscdb` (and per-workspace `state.vscdb`), SQLite, table `cursorDiskKV` with keys `composerData:<id>`, `bubbleId:<chat>:<msg>`, plus `agentKv`, `checkpointId`; `ItemTable` holds login tokens (do not read). Bubble rows once carried a `tokenCount`; a third-party adapter (codewrapped PR #6) states Cursor kept it locally "only until early 2026" so Cursor tokens are partial; vibe-replay calls aggregate totals a lower bound. tokscale explicitly "does not treat the desktop SQLite DB as a usage ledger" and does not parse `~/.cursor` CLI state. The DB is huge and volatile (reports of 97 GB growth and of a Cursor update wiping `cursorDiskKV`), WAL-locked while Cursor runs.

**Server side.** Per-request model + tokens exist at `cursor.com/api/dashboard/get-filtered-usage-events` and the `GetAggregatedUsageEvents` RPC (see `docs/research/cursor.md` section 5); tokscale uses this API, with its own cache under `~/.config/tokscale/cursor-cache/`.

**Caveats.** Tokens in `state.vscdb` login keys may be encrypted (`cursor-safe-storage:v1:`, see cursor.md); chat content is stored unencrypted but is private (prompts, code). Cursor's ToS for the private dashboard endpoints was not checked.

**Verdict: not feasible as a local scan.** If SubHub wants Cursor tokens it must call the usage-events API with the user's token (separate, riskier feature, already scoped in cursor.md). A pure local approximation (IDE `tokenCount` for old chats) is not worth building.

## 3. Google Antigravity `agy` (and IDE)

**Locations.** `GEMINI_CLI_HOME` replaces `~/.gemini`.
- CLI: `%USERPROFILE%\.gemini\antigravity-cli\conversations\*.db` (SQLite, one per conversation, named by conversation id).
- IDE extension: `~/.gemini/antigravity/conversations/*.db`; ccusage also scans `antigravity-ide/`, `antigravity-backup/`, `antigravity-acp/` and `~/.config/antigravity/conversations/`; `ANTIGRAVITY_DATA_DIR` (comma-separated) overrides in ccusage.
- Plain-text transcript: `~/.gemini/antigravity-cli/brain/<conversation-id>/.system_generated/logs/transcript.jsonl` (+ `transcript_full.jsonl`); one step per line with `step_index`, `source`, `type`, `status`, `created_at`, `content`, `tool_calls[]`, `thinking`. It has timestamps but NO tokens, no cwd, and the model only by inference from a settings-change line (checked on 1.1.14 by one guide). `history.jsonl` indexes conversations [3p].
- Official docs (antigravity.google/docs/cli/conversations) document no paths or formats; only that histories are scoped to the working directory. On this PC `%USERPROFILE%\.gemini` does not exist.

**Token data.** SQLite table `gen_metadata(idx, data)`; `data` is a protobuf blob. Reverse-engineered usage fields [3p, CodexBar docs, citing ccusage and Tokscale]: field 1 = model enum id (e.g. 1298 = `gemini-3.7-flash`), 2 = input, 5 = cache read, 9 = reasoning output, 10 = text output; timestamp at `chatModel.#9.#4` (seconds/nanos), fallback via a `steps` table (`steps.metadata`). ccusage merges generation metadata and step metadata, de-duplicates by response/message id, reports fresh input, cache read/write, visible output and reasoning, normalizes model names, and inherits the last seen model for continuation rows. Interpretation of fields 9/10 differs between Tokscale and ccusage/CodexBar. An older tool (antigravity-usage) claimed tokens are not persisted at all and only estimated; the newer tools show they exist in current agy versions (CodexBar notes the 1.1.18 timestamp layout is unsupported). cwd/project: not confirmed in any source.
- The model enum needs a hand-maintained id-to-name table; unknown ids must show as "unknown".
- Databases may be WAL-mode and locked while agy runs: open read-only, retry with `immutable=1` only if no `-wal` exists (CodexBar's approach); never copy.

**Tooling.** ccusage (`ccusage antigravity daily|monthly|session`), CodexBar (cost provider), Tokscale (SQLite for CLI/extension; the editor path uses a live RPC against the local language server, macOS/Linux only), UsageAtlas.

**Stability.** Internal, undocumented, drifts per agy version (CodexBar treats unknown columns as schema drift and reports a lower bound). SubHub would need `rusqlite` (read-only) plus a small hand-written protobuf wire parser (no schema file); no new network access.

**Terms of service.** Antigravity Additional Terms item 6 (as quoted by third parties; zentor.ai): "Using third party software, tools, or services to access the Service (e.g. using OpenClaw with Antigravity OAuth) is a breach of this Agreement", grounds for suspending Antigravity/Gemini CLI accounts. The reported suspensions concern routing inference through the Antigravity OAuth token; the article cites none for local-only tooling. Reading the user's own `.db` files uses no token and makes no request to the service, so it is a different question, but Google has not said so explicitly. Keep it strictly local, read-only, and do not combine it with calls using the Antigravity OAuth client.

**Verdict: feasible but fragile**, do it last, label as "estimate, local data".

## 4. Suggested order and display

1. Copilot CLI `events.jsonl` (JSONL fits the existing scanner; needs shutdown-snapshot differencing per session+model, bucket at shutdown time, `unknown` model fallback, ignore unknown event types). Show a note "sessions still running or crashed are not counted yet".
2. Antigravity `agy` SQLite+protobuf, optional, behind a "beta" label.
3. Cursor: no local scan; only revisit as an API-based feature.
4. VS Code Copilot Chat: later, after the CLI works.

Cost estimates: Copilot also logs its own credit total (`totalNanoAiu`, 1 credit = $0.01), a better number than a token-price estimate for Copilot; keep it as a second figure. Cursor and Antigravity have no authoritative local cost; use the API price table and label "estimate".

## Sources

- https://ccusage.com/guide/copilot/ (Copilot CLI paths, `session.shutdown`, field semantics, `COPILOT_HOME`)
- https://ccusage.com/guide/antigravity/ (agy `.db` roots, `ANTIGRAVITY_DATA_DIR`, token categories)
- https://docs.github.com/en/copilot/how-tos/copilot-sdk/features/streaming-events (official: `session.shutdown`, `session.usage_checkpoint`, ephemeral `assistant.usage`)
- https://docs.github.com/en/copilot/concepts/agents/copilot-cli/chronicle (official: `~/.copilot/session-state/`, session store, `/chronicle reindex`)
- https://github.com/github/copilot-cli/issues/3551 (events.jsonl undocumented, no stability contract)
- https://github.com/getagentseal/codeburn/issues/944 (`session.start` fields, shutdown is the only source of input/cache tokens)
- https://github.com/ccusage/ccusage/issues/1174 (Copilot session-state adapter)
- https://usekibble.com/blog/where-github-copilot-keeps-its-usage-logs (Windows paths, VS Code chat session format)
- https://github.com/junhoyeo/tokscale (Copilot `session-store.db`; Cursor via API only; Antigravity variants)
- https://allaboutcoding.ghinda.com/where-ai-coding-clis-store-session-logs/ (Cursor CLI `chats/` layout, undocumented)
- https://github.com/ponbac/token-ledger/issues/21 (Cursor Agent writes no token usage to disk)
- https://github.com/dayamjz/assistant/issues/510 and /pull/512 (Cursor CLI JSON `usage` block)
- https://github.com/SergeyGalaxyOrsik/codewrapped/pull/6 (Cursor IDE `tokenCount` only until early 2026)
- https://vibe-replay.com/blog/cursor-local-storage/ and https://forum.cursor.com/t/state-vscdb-grew-from-3-1-gb-97-gb-macos/166013 (Cursor `cursorDiskKV` layout, size/volatility)
- https://github.com/steipete/CodexBar/blob/main/docs/antigravity.md (agy SQLite/protobuf fields, timestamps, schema-drift handling)
- https://jazzyalex.github.io/agent-sessions/guides/antigravity-cli-local-history.html (agy `transcript.jsonl` fields, no tokens)
- https://antigravity.google/docs/cli/conversations/ (official: no storage details)
- https://zentor.ai/blog/antigravity-account-suspended-third-party (Antigravity terms item 6, suspensions concerned OAuth inference use)
- Earlier notes: `docs/research/copilot.md`, `docs/research/cursor.md`, `docs/research/gemini-antigravity.md`
