# Plan prices and capacity factors: Copilot, Cursor, Antigravity

As of 2026-11-09. List prices in USD per month, monthly billing, excluding tax. Feeds `PlanTier` in `crates/subhub-core/src/insights/fit.rs` (factor = capacity relative to the provider's smallest paid tier).
"exact" = factor from a documented allowance ratio. "approx" = marketing "Nx" wording or a guess. [?] = uncertain / not verified against a primary source.

## 1. GitHub Copilot (individual)

Since 2026-06-01 monthly plans use GitHub AI Credits (1 credit = $0.01). Allowance = base (equals the price, fixed) + flex (variable, "may change over time"). Credits reset 00:00 UTC on the 1st of each calendar month, no rollover.

| Tier | Label SubHub reads (`parse_plan`) | USD/mo | Monthly credits (base + flex) | Factor vs Pro | Exact/approx | In advice |
|---|---|---|---|---|---|---|
| Free | `Free` (`access_type_sku` = `free_limited_copilot`) | 0 | not stated ("limited"), 2,000 completions | n/a | n/a | no |
| Pro | `Pro` | 10 | 1,500 (1,000 + 500) | 1.0 | exact | yes (smallest paid) |
| Pro+ | `Pro+` | 39 | 7,000 (3,900 + 3,100) | 4.67 | exact on totals (GitHub says "4x+") | yes |
| Max | `Max` | 100 | 20,000 (10,000 + 10,000) | 13.33 | exact on totals (GitHub says "2.9x+" vs Pro+) | yes |

- Base-only ratios would be 1 : 3.9 : 10. Use totals (what the quota endpoint's `entitlement` shows today) but treat them as exact only while flex is unchanged. Re-read `entitlement` from the endpoint instead of hardcoding when possible.
- Annual price: not shown on the plans page [?] (annual Pro/Pro+ are a legacy arrangement, see below). Max is an upgrade for Student/Pro/Pro+ subscribers.
- Legacy: annual Pro/Pro+ subscribers stay on premium-request billing until the plan expires (then drop to Free; model multipliers went up on 2026-06-01 for them). Window is still monthly. Request allowances for them were not verified [?]. Exclude from advice (different unit, advice would mislead; they end soon).
- Exclude: Free, Student/Education (`copilot_plan: individual_edu`, `access_type_sku: free_educational_quota`), trials (`trial_30_monthly_subscriber_quota`), Business ($19/user, pooled) and Enterprise ($39/user, pooled), any seat assigned by an org (it cancels the personal plan).

### What SubHub sees
`GET api.github.com/copilot_internal/user`: `copilot_plan` + `access_type_sku`. Observed values (all third-party, none documented by GitHub):

| `copilot_plan` | `access_type_sku` | Current `parse_plan` | Tier |
|---|---|---|---|
| `individual` | `free_limited_copilot` | Free | none |
| `individual` | `monthly_subscriber_quota` | Pro (via `plan == individual`) | ambiguous: seen for a paid seat; Pro+/Max SKU strings not found [?] |
| `individual` | `trial_30_monthly_subscriber_quota` | Pro | trial, none |
| `individual_edu` | `free_educational_quota` | Free (contains "free") | education, none |
| `business` / `enterprise` | not found [?] | Business / Enterprise | none |
| any | `subscription_ended` | (failure, no plan) | none |

Findings for the code:
- The SKU strings for Pro+ and Max are NOT confirmed. `parse_plan` maps any SKU containing `plus`/`max` and falls back to Pro for `copilot_plan == individual`, so an unknown Pro+/Max SKU would silently read as Pro and give wrong advice.
- Safer: infer the tier from the credit entitlement in `quota_snapshots.premium_interactions.entitlement` (1,500 -> Pro, 7,000 -> Pro+, 20,000 -> Max) when `token_based_billing` is true, and show no tier advice for any other value (flex changes, org seats, legacy annual).
- `free_educational_quota` is mislabelled "Free" by the substring `free`; harmless for advice (no tier) but the label is wrong.

## 2. Cursor

Two lineups coexist. Current ("The X Subscription"): one pool for Cursor models, refreshes weekly, billed on top for third-party models at API price (prepaid credit from Premium up; Plus is limited to Grok models). Legacy: two monthly pools.

### Current (weekly)
| Tier | Label SubHub reads | USD/mo | Allowance | Factor vs Plus | Exact/approx | In advice |
|---|---|---|---|---|---|---|
| Free | `Free` / `free` / `hobby` [?] | 0 | "Limited" | n/a | n/a | no |
| Plus | `Plus` [?] | 8 | base shared pool | 1 | exact (defines the unit) | yes |
| Premium | `Premium` [?] | 30 | "4x Plus" | 4 | exact as published by Cursor | yes |
| Super | `Super` [?] | 100 | "15x Plus" | 15 | exact as published | yes |
| Ultra | `Ultra` (membership `unified_ultra`) | 200 | "50x Plus" | 50 | exact as published | yes |

Annual: $80 / $300 / $1,000 / $2,000 (docs page; the pricing page lists none). Teams seats ($30/$100/$200 per seat) mirror the same tiers: exclude.

### Legacy (monthly pools, reset with the billing cycle)
| Tier | Label SubHub reads | USD/mo | Allowance | Factor vs Pro | Exact/approx | In advice |
|---|---|---|---|---|---|---|
| Hobby | `Hobby` / `free` | 0 | "limited" | n/a | n/a | no |
| Start (India only) | `Start` | INR 649 | no Other Models pool | n/a | n/a | no |
| Pro | `Pro` | 20 | Cursor Models + Other Models pools | 1 | defines unit | yes, within legacy ladder only |
| Pro+ | `Pro+` / `pro_plus` | 60 | "more of each pool than Pro" | about 3 | approx (Cursor gives no number; third parties list Other-Models $20 / $60-70 / $400 [?]) | yes, within legacy ladder only |
| Ultra | `Ultra` (membership `ultra`) | 200 | "more than Pro+" | about 20 | approx (same reason) | yes, within legacy ladder only |
| Teams Standard / Premium | `team` | 40 / 120 per user | Premium = 5x Standard | 5 | exact | no (team) |

- Legacy and current Ultra are both $200 and both called "Ultra". They differ in window: tell them apart by the span of `billingCycleStart..billingCycleEnd` (about 7 days = current/weekly, about 30 days = legacy/monthly), as already noted in `docs/research/cursor.md`. The `unified_` prefix on `membershipType` also marks the new lineup [?].
- Do not suggest crossing lineups: switching from legacy to the new lineup is permanent (docs/third parties), and legacy third-party usage is included while on the new plans it is billed separately. Advise only within each ladder. Cursor's own mapping: Pro -> Premium, Pro+ -> Super, Ultra -> Ultra (not a capacity-equal mapping).
- Price per capacity (current): Plus $8/1x, Premium $7.50 per x, Super $6.67 per x, Ultra $4 per x.

### What SubHub sees
`GetPlanInfo.planInfo.planName` (passed through as the label by `cursor::parse_plan`). The exact strings are not documented anywhere found; `usage-summary.membershipType` has been seen as pro, pro_plus, ultra, free_trial, hobby, free, express, team, business, enterprise, `unified_*`. Names in the table above for the current lineup are inferred from the pricing page [?]. A plan-name-only mapping cannot separate legacy Ultra from current Ultra: also use the cycle span. Unknown names: no tier.

## 3. Google Antigravity (Google AI plans)

Antigravity has no plan of its own: quota comes from the Google AI subscription. Pro and Ultra: 5-hour windows under a weekly cap; Free (and Plus, which lists no Antigravity benefit): weekly only. Pro/Ultra can add paid AI credits for overage.

| Tier | Label SubHub reads | USD/mo | Allowance | Factor vs Pro | Exact/approx | In advice |
|---|---|---|---|---|---|---|
| Free | `Free` (weekly-only window; the only plan string `antigravity.rs` sets) | 0 | "weekly rate limit" | n/a | n/a | no |
| AI Plus | none (no 5h window, likely reads as `Free`) [?] | 4.99 | no Antigravity benefit listed | n/a | n/a | no |
| AI Pro | none today (5h window present) | 19.99 | "entry" rate limits | 1 | defines unit | yes, capacity only |
| AI Ultra 5x | none today | 99.99 | "5x higher usage limits vs. AI Pro" | 5 | approx for Antigravity (the 5x is stated for Gemini usage; Antigravity says "higher") | yes, capacity only |
| AI Ultra 20x | none today | 199.99 | "20x higher usage limits vs. AI Pro" | 20 | approx (same) | yes, capacity only |

- Annual: Pro $199.99/yr per a third-party quote of Google's page [?]; Ultra is monthly only. Google's US page lists no annual price.
- Price fairness: every paid plan bundles storage (Plus 400 GB, Pro 5 TB, Ultra 20 TB+/30 TB [?]), YouTube Premium (Lite/full), Gemini app limits and other perks. No Google-published "AI part" price exists. Recommendation: show capacity advice (too big / fits / at limit and the next tier by name and its list price) but NO savings figure and no "cancel and save" line, because dropping to Free or Plus loses storage and perks. Do not compute price_change for Antigravity. If a number is ever needed, show list price differences only, labelled "whole plan, incl. storage".
- Exclude: Free, Plus, Workspace / Gemini Enterprise ($30/seat) and Cloud-billed accounts, education/promo offers, and the Gemini CLI consumer tiers (retired 2026-06-18).
- Plan strings: `agy /usage` JSON carries no plan or identity (see `docs/research/gemini-antigravity.md`); `antigravity.rs` only infers `Free` when there is no 5h bucket. Paid tier could be read from `loadCodeAssist.paidTier.name` (reported as `Google AI Pro` in a made-up example from CodexBar docs [?], Ultra 5x/20x label not found). Until then Pro/Ultra size can only be guessed from nothing: no advice without a known tier.

## Open uncertainty
- Copilot: Pro+/Max `access_type_sku` strings; Business/Enterprise strings; legacy annual request allowances; whether flex credits stay.
- Cursor: exact `planName` strings; legacy Pro+/Ultra multipliers (Cursor states none now); whether Plus is shown as "Plus".
- Antigravity: no plan label for paid tiers from `agy /usage`; real Antigravity multipliers (only qualitative); Ultra 20x storage (20 vs 30 TB differs by page); Google US pages gave no annual prices.

## Sources
- Copilot plans (prices, credits, "4x+", "2.9x+"): https://github.com/features/copilot/plans
- Copilot individual billing (base/flex, reset): https://docs.github.com/en/copilot/concepts/billing-and-usage/individuals/billing
- Usage-based billing announcement (annual plans, Business $19, Enterprise $39): https://github.blog/news-insights/company-news/github-copilot-is-moving-to-usage-based-billing/ ; changelog https://github.blog/changelog/2026-06-01-updates-to-github-copilot-billing-and-plans/ ; legacy annual https://docs.github.com/en/copilot/reference/copilot-billing/request-based-billing-legacy
- SKU values (third-party): https://github.com/yetone/magpie/pull/510 , https://github.com/NousResearch/hermes-agent/pull/18783 , https://github.com/suppressio/ia-hypermiler/issues/7 ; `subscription_ended`: https://github.com/steipete/CodexBar/blob/main/docs/copilot.md
- Cursor pricing (current lineup, Nx Plus, annual, weekly): https://cursor.com/docs/account/pricing , https://cursor.com/pricing
- Cursor legacy plans (prices, monthly pools): https://cursor.com/docs/account/pricing/legacy-cursor-plans ; third-party pool dollar figures: https://www.nocode.mba/articles/cursor-pricing
- Antigravity plans (qualitative quotas): https://antigravity.google/docs/plans
- Google AI plan prices, Antigravity rate-limit tiers, 5x/20x vs Pro: https://gemini.google/us/subscriptions/?hl=en ; I/O 2026 overview: https://the-decoder.com/google-overhauls-its-ai-subscriptions-at-i-o-2026-with-three-tiers-starting-at-10-a-month/ ; Pro annual (third party): https://sessionwatcher.com/guides/antigravity-pricing
