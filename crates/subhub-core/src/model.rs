//! Domain types shared by core and app. Words follow CONTEXT.md.
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderKind {
    Claude,
    Codex,
}

/// Outcome of one usage check. Missing data is `Unavailable`, never zero or full.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsageState {
    Ready,
    Unavailable,
    SignInRequired,
    /// Provider throttling; carries a provider cooldown that manual retry must not bypass.
    RateLimited,
    CliMissing,
}

/// One allowance window. `key` is stable across readings (e.g. "five_hour",
/// "seven_day_opus", "codex:primary") and is what pins/hides refer to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AllowanceWindow {
    pub key: String,
    /// Display label, e.g. "Session", "Weekly", "Weekly · Opus".
    pub label: String,
    /// 0..=100 as reported by the provider.
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

impl AllowanceWindow {
    pub fn remaining_percent(&self) -> f64 {
        (100.0 - self.used_percent).clamp(0.0, 100.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub fetched_at: DateTime<Utc>,
    pub windows: Vec<AllowanceWindow>,
}

/// Where readings come from (ADR 0001: hub first, profile as fallback).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadSource {
    Hub,
    Profile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageResult {
    pub state: UsageState,
    /// Short, user-facing; never contains tokens, raw responses or account identity.
    pub message: String,
    pub reading: Option<Reading>,
    /// Provider cooldown (e.g. Retry-After).
    pub retry_after_secs: Option<u64>,
    pub email: Option<String>,
    /// e.g. "Max 20x", "Pro", "Plus".
    pub plan: Option<String>,
    pub source: ReadSource,
}

/// Link from an account to a hub login. The hub owns the token; we only hold its index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HubLink {
    /// Opaque value from `auth-files` (kept as JSON: the hub may use a number or a string).
    pub auth_index: serde_json::Value,
    /// Auth file name, stable id for re-matching after restarts.
    pub file_name: String,
    /// Disabled in the hub: shown as Benched.
    pub benched: bool,
    /// ChatGPT account id (Codex hub logins); sent as `Chatgpt-Account-Id` for usage reads.
    #[serde(default)]
    pub account_id: Option<String>,
    /// Credential priority in the hub as last listed.
    #[serde(default)]
    pub priority: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: Uuid,
    pub provider: ProviderKind,
    /// User-editable display label (1..=60 chars); not proof of identity.
    pub label: String,
    /// Login email when known; the merge key between profile and hub.
    pub email: Option<String>,
    /// CLI profile folder (e.g. ~/.claude-work). None for hub-only accounts.
    pub profile_dir: Option<PathBuf>,
    pub hub: Option<HubLink>,
    pub group: Option<String>,
    pub hidden: bool,
    /// Window keys hidden for this account.
    pub hidden_windows: Vec<String>,
    /// Window keys pinned to the Priority view.
    pub pinned_windows: Vec<String>,
}

impl Account {
    pub fn is_merged(&self) -> bool {
        self.profile_dir.is_some() && self.hub.is_some()
    }
}
