//! Plan labels. Only known values are shown; unknown provider values never leak into the UI.
use serde_json::Value;

use crate::util::text;

pub(crate) fn label(value: &str) -> Option<String> {
    let l = match value.to_ascii_lowercase().as_str() {
        "free" => "Free",
        "pro" => "Pro",
        "plus" => "Plus",
        "max" => "Max",
        "team" => "Team",
        "enterprise" => "Enterprise",
        "edu" => "Edu",
        "go" => "Go",
        // Codex plan ids (Codex CLI `KnownPlan`, 2026-11-08); no prices for these, so no plan fit.
        "prolite" => "Pro Lite",
        "promax" => "Pro Max",
        "business" | "self_serve_business_prolite" | "self_serve_business_usage_based" => {
            "Business"
        }
        "ent26" | "enterprise_cbp_automation" | "enterprise_cbp_usage_based" => "Enterprise",
        "edu_plus" => "Edu Plus",
        "edu_pro" => "Edu Pro",
        _ => return None,
    };
    Some(l.to_string())
}

/// "max" plus a rate-limit tier gives "Max 5x" / "Max 20x"; everything else a plain label.
pub(crate) fn claude_label(plan: &str, tier: Option<&str>) -> Option<String> {
    if plan.eq_ignore_ascii_case("max") {
        return Some(
            match tier {
                Some("default_claude_max_5x") => "Max 5x",
                Some("default_claude_max_20x") => "Max 20x",
                _ => "Max",
            }
            .to_string(),
        );
    }
    label(plan)
}

/// Claude `oauth/profile` body (`organization.organization_type`) or credential metadata
/// (`claudeAiOauth.subscriptionType`).
pub(crate) fn claude_plan(v: &Value) -> Option<String> {
    if let Some(org) = v.get("organization").filter(|o| o.is_object()) {
        let plan = match text(org, "organization_type")? {
            "claude_max" => "max",
            "claude_pro" => "pro",
            "claude_team" => "team",
            "claude_enterprise" => "enterprise",
            _ => return None,
        };
        return claude_label(plan, text(org, "rate_limit_tier"));
    }
    let oauth = v.get("claudeAiOauth").filter(|o| o.is_object())?;
    claude_label(
        text(oauth, "subscriptionType")?,
        text(oauth, "rateLimitTier"),
    )
}

#[cfg(test)]
mod label_tests {
    use super::label;

    #[test]
    fn codex_plan_ids_get_labels() {
        assert_eq!(label("prolite").as_deref(), Some("Pro Lite"));
        assert_eq!(label("promax").as_deref(), Some("Pro Max"));
        assert_eq!(
            label("self_serve_business_usage_based").as_deref(),
            Some("Business")
        );
        assert_eq!(label("ent26").as_deref(), Some("Enterprise"));
        assert_eq!(label("PRO").as_deref(), Some("Pro"));
        assert_eq!(label("guest"), None);
    }
}
