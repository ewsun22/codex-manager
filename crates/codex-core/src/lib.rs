//! Privacy-preserving, cross-platform normalization primitives.
//!
//! The parser deliberately emits metadata only. It never retains message text,
//! authorization values, or arbitrary event payloads.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_write_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub total_tokens: i64,
}

impl TokenUsage {
    /// A single Codex request cannot legitimately consume an unbounded number
    /// of tokens. Keeping the bound well below `i64::MAX` also leaves headroom
    /// for all supported aggregate paths.
    pub const MAX_FIELD_VALUE: i64 = 1_000_000_000_000;
    /// Per model-call budget. Cumulative session counters may legitimately be
    /// much larger, but one normalized call above this ceiling is treated as
    /// corrupt input instead of contaminating cost and dashboard aggregates.
    pub const MAX_CALL_FIELD_VALUE: i64 = 10_000_000;

    pub fn is_zero(&self) -> bool {
        self == &Self::default()
    }
    pub fn is_valid(&self) -> bool {
        [
            self.input_tokens,
            self.cached_input_tokens,
            self.cache_write_input_tokens,
            self.output_tokens,
            self.reasoning_output_tokens,
            self.total_tokens,
        ]
        .into_iter()
        .all(|value| (0..=Self::MAX_FIELD_VALUE).contains(&value))
    }
    pub fn is_consistent(&self) -> bool {
        self.cached_input_tokens
            .checked_add(self.cache_write_input_tokens)
            .is_some_and(|cached| cached <= self.input_tokens)
            && self.reasoning_output_tokens <= self.output_tokens
            && self
                .input_tokens
                .checked_add(self.output_tokens)
                .is_some_and(|total| total == self.total_tokens)
    }
    pub fn is_valid_call(&self) -> bool {
        self.is_valid()
            && self.is_consistent()
            && [
                self.input_tokens,
                self.cached_input_tokens,
                self.cache_write_input_tokens,
                self.output_tokens,
                self.reasoning_output_tokens,
                self.total_tokens,
            ]
            .into_iter()
            .all(|value| value <= Self::MAX_CALL_FIELD_VALUE)
    }
    pub fn monotonic_from(&self, previous: &Self) -> bool {
        self.input_tokens >= previous.input_tokens
            && self.cached_input_tokens >= previous.cached_input_tokens
            && self.cache_write_input_tokens >= previous.cache_write_input_tokens
            && self.output_tokens >= previous.output_tokens
            && self.reasoning_output_tokens >= previous.reasoning_output_tokens
            && self.total_tokens >= previous.total_tokens
    }
    pub fn checked_delta(&self, previous: &Self) -> Option<Self> {
        let delta = Self {
            input_tokens: self.input_tokens.checked_sub(previous.input_tokens)?,
            cached_input_tokens: self
                .cached_input_tokens
                .checked_sub(previous.cached_input_tokens)?,
            cache_write_input_tokens: self
                .cache_write_input_tokens
                .checked_sub(previous.cache_write_input_tokens)?,
            output_tokens: self.output_tokens.checked_sub(previous.output_tokens)?,
            reasoning_output_tokens: self
                .reasoning_output_tokens
                .checked_sub(previous.reasoning_output_tokens)?,
            total_tokens: self.total_tokens.checked_sub(previous.total_tokens)?,
        };
        delta.is_valid().then_some(delta)
    }
    pub fn checked_add(&self, other: &Self) -> Option<Self> {
        let total = Self {
            input_tokens: self.input_tokens.checked_add(other.input_tokens)?,
            cached_input_tokens: self
                .cached_input_tokens
                .checked_add(other.cached_input_tokens)?,
            cache_write_input_tokens: self
                .cache_write_input_tokens
                .checked_add(other.cache_write_input_tokens)?,
            output_tokens: self.output_tokens.checked_add(other.output_tokens)?,
            reasoning_output_tokens: self
                .reasoning_output_tokens
                .checked_add(other.reasoning_output_tokens)?,
            total_tokens: self.total_tokens.checked_add(other.total_tokens)?,
        };
        total.is_valid().then_some(total)
    }
    /// Returns false without modifying `self` when the aggregate is outside
    /// the supported token range. Existing callers can deliberately ignore the
    /// boolean, while parser/storage boundaries use it to mark usage unavailable.
    pub fn add(&mut self, other: &Self) -> bool {
        let Some(total) = self.checked_add(other) else {
            return false;
        };
        *self = total;
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub session_id: String,
    pub thread_id: String,
    pub cli_version: Option<String>,
    pub cwd: Option<String>,
    pub model_provider: Option<String>,
    pub started_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub turn_id: String,
    pub session_id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub provider: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub first_visible_output_ms: Option<i64>,
    pub status: String,
    pub usage: TokenUsage,
    pub model_call_count: i64,
    pub usage_confidence: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCall {
    pub event_key: String,
    pub session_id: String,
    pub turn_id: String,
    pub ordinal: Option<i64>,
    pub occurred_at: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub provider: Option<String>,
    pub usage: TokenUsage,
    pub cumulative_usage: TokenUsage,
    pub usage_confidence: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineItem {
    pub event_key: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub ordinal: Option<i64>,
    pub occurred_at: Option<String>,
    pub item_type: String,
    pub role: Option<String>,
    pub phase: Option<String>,
    pub tool_name: Option<String>,
    pub content_utf8_bytes: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub ordinal: Option<i64>,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub session: Option<Session>,
    pub turns: Vec<Turn>,
    pub model_calls: Vec<ModelCall>,
    pub timeline: Vec<TimelineItem>,
    pub diagnostics: Vec<Diagnostic>,
    pub unhandled_event_counts: BTreeMap<String, u64>,
    pub ignored_duplicate_events: u64,
    pub ignored_duplicate_usage_snapshots: u64,
}

/// Enough parser state to continue from a verified JSONL byte offset without
/// retaining prior message payloads or replaying the whole file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeState {
    pub session: Option<Session>,
    pub current_turn: Option<Turn>,
    pub last_cumulative_usage: Option<TokenUsage>,
    #[serde(default)]
    pub unavailable_turn_id: Option<String>,
}

#[derive(Default)]
pub struct RolloutNormalizer {
    session: Option<Session>,
    current_turn_id: Option<String>,
    turns: HashMap<String, Turn>,
    model_calls: Vec<ModelCall>,
    timeline: Vec<TimelineItem>,
    diagnostics: Vec<Diagnostic>,
    unhandled: BTreeMap<String, u64>,
    seen: HashSet<String>,
    last_usage: Option<TokenUsage>,
    duplicate_events: u64,
    duplicate_usage: u64,
    dirty_turns: HashSet<String>,
    unavailable_usage_turns: HashSet<String>,
}

impl RolloutNormalizer {
    pub fn from_resume_state(state: ResumeState) -> Self {
        let mut turns = HashMap::new();
        let current_turn_id = state.current_turn.as_ref().map(|turn| turn.turn_id.clone());
        if let Some(turn) = state.current_turn {
            turns.insert(turn.turn_id.clone(), turn);
        }
        let mut unavailable_usage_turns = HashSet::new();
        if let Some(turn_id) = state.unavailable_turn_id {
            unavailable_usage_turns.insert(turn_id);
        }
        Self {
            session: state.session,
            current_turn_id,
            turns,
            last_usage: state.last_cumulative_usage,
            unavailable_usage_turns,
            ..Self::default()
        }
    }

    pub fn resume_state(&self) -> ResumeState {
        ResumeState {
            session: self.session.clone(),
            current_turn: self
                .current_turn_id
                .as_ref()
                .and_then(|id| self.turns.get(id))
                .cloned(),
            last_cumulative_usage: self.last_usage.clone(),
            unavailable_turn_id: self
                .current_turn_id
                .as_ref()
                .filter(|id| self.unavailable_usage_turns.contains(*id))
                .cloned(),
        }
    }

    /// Returns only the projection changed since construction or resume.
    /// Model calls and timeline items are already event-level incremental.
    pub fn incremental_snapshot(&self) -> Snapshot {
        let mut turns = self
            .dirty_turns
            .iter()
            .filter_map(|id| self.turns.get(id))
            .cloned()
            .collect::<Vec<_>>();
        turns.sort_by(|a, b| a.turn_id.cmp(&b.turn_id));
        Snapshot {
            session: self.session.clone(),
            turns,
            model_calls: self.model_calls.clone(),
            timeline: self.timeline.clone(),
            diagnostics: self.diagnostics.clone(),
            unhandled_event_counts: self.unhandled.clone(),
            ignored_duplicate_events: self.duplicate_events,
            ignored_duplicate_usage_snapshots: self.duplicate_usage,
        }
    }

    pub fn parse_line(&mut self, line: &str, offset: Option<u64>) {
        let raw: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => {
                self.diag("invalid_json", "warning", None, "跳过无法解析的 JSONL 行。");
                return;
            }
        };
        let Some(obj) = raw.as_object() else {
            self.diag(
                "invalid_envelope",
                "warning",
                None,
                "跳过不符合 envelope 的事件。",
            );
            return;
        };
        let Some(event_type) = obj.get("type").and_then(Value::as_str) else {
            self.diag(
                "invalid_envelope",
                "warning",
                None,
                "跳过不符合 envelope 的事件。",
            );
            return;
        };
        let Some(payload) = obj.get("payload").and_then(Value::as_object) else {
            self.diag(
                "invalid_envelope",
                "warning",
                None,
                "跳过不符合 envelope 的事件。",
            );
            return;
        };
        let ordinal = obj.get("ordinal").and_then(Value::as_i64);
        let timestamp = obj
            .get("timestamp")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        if event_type == "session_meta" {
            self.parse_session(payload, ordinal, timestamp, offset);
            return;
        }
        if self.session.is_none() {
            self.diag(
                "missing_session",
                "warning",
                ordinal,
                "跳过 session metadata 之前的事件。",
            );
            return;
        }
        let event_key = self.event_key(event_type, payload, ordinal, timestamp.as_deref(), offset);
        if !self.seen.insert(event_key.clone()) {
            self.duplicate_events += 1;
            return;
        }
        match event_type {
            "turn_context" => self.parse_context(payload),
            "response_item" => self.parse_item(payload, event_key, ordinal, timestamp),
            "event_msg" => self.parse_event(payload, event_key, ordinal, timestamp),
            _ => self.count(format!("top:{}", safe_label(event_type))),
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        let mut turns = self.turns.values().cloned().collect::<Vec<_>>();
        turns.sort_by(|a, b| a.turn_id.cmp(&b.turn_id));
        Snapshot {
            session: self.session.clone(),
            turns,
            model_calls: self.model_calls.clone(),
            timeline: self.timeline.clone(),
            diagnostics: self.diagnostics.clone(),
            unhandled_event_counts: self.unhandled.clone(),
            ignored_duplicate_events: self.duplicate_events,
            ignored_duplicate_usage_snapshots: self.duplicate_usage,
        }
    }
    fn parse_session(
        &mut self,
        p: &serde_json::Map<String, Value>,
        ordinal: Option<i64>,
        timestamp: Option<String>,
        offset: Option<u64>,
    ) {
        let sid = val_str(p, "session_id")
            .or_else(|| val_str(p, "id"))
            .or_else(|| val_str(p, "thread_id"));
        let Some(session_id) = sid else {
            self.diag(
                "invalid_envelope",
                "error",
                ordinal,
                "Session metadata 未包含标识符。",
            );
            return;
        };
        self.session = Some(Session {
            thread_id: val_str(p, "thread_id")
                .or_else(|| val_str(p, "id"))
                .unwrap_or_else(|| session_id.clone()),
            session_id,
            cli_version: val_str(p, "cli_version"),
            cwd: val_str(p, "cwd"),
            model_provider: val_str(p, "model_provider"),
            started_at: val_str(p, "timestamp").or(timestamp),
        });
        let key = self.event_key("session_meta", p, ordinal, None, offset);
        if !self.seen.insert(key) {
            self.duplicate_events += 1;
        }
    }
    fn parse_context(&mut self, p: &serde_json::Map<String, Value>) {
        let id = val_str(p, "turn_id").or_else(|| self.current_turn_id.clone());
        let Some(id) = id else { return };
        self.current_turn_id = Some(id.clone());
        let turn = self.turn(&id);
        if let Some(v) = val_str(p, "model") {
            turn.model = Some(v)
        }
        if let Some(v) = val_str(p, "effort") {
            turn.effort = Some(v)
        }
        if let Some(v) = val_str(p, "cwd") {
            turn.cwd = Some(v)
        }
        // A call keeps the context known when it occurred. Retrospectively
        // changing only this batch would make enrichment depend on checkpoint
        // boundaries, and could overwrite an earlier call's explicit model.
        // Storage resolves missing call fields against the canonical turn.
        self.dirty_turns.insert(id);
    }
    fn parse_item(
        &mut self,
        p: &serde_json::Map<String, Value>,
        key: String,
        ordinal: Option<i64>,
        timestamp: Option<String>,
    ) {
        let item_type = val_str(p, "type").unwrap_or_else(|| "unknown".into());
        let bytes = if item_type == "message" {
            message_bytes(p.get("content"))
        } else {
            None
        };
        self.timeline.push(TimelineItem {
            event_key: key,
            session_id: self.session.as_ref().unwrap().session_id.clone(),
            turn_id: self.current_turn_id.clone(),
            ordinal,
            occurred_at: timestamp,
            item_type,
            role: val_str(p, "role"),
            phase: val_str(p, "phase"),
            tool_name: val_str(p, "name"),
            content_utf8_bytes: bytes,
        });
    }
    fn parse_event(
        &mut self,
        p: &serde_json::Map<String, Value>,
        key: String,
        ordinal: Option<i64>,
        timestamp: Option<String>,
    ) {
        match p.get("type").and_then(Value::as_str) {
            Some("task_started") => {
                let Some(id) = val_str(p, "turn_id") else {
                    self.diag(
                        "missing_turn",
                        "warning",
                        ordinal,
                        "task_started 缺少 turn。",
                    );
                    return;
                };
                self.current_turn_id = Some(id.clone());
                self.unavailable_usage_turns.remove(&id);
                let t = self.turn(&id);
                t.started_at = val_str(p, "started_at").or(timestamp);
                t.status = "running".into();
                self.dirty_turns.insert(id);
            }
            Some("token_count") => self.parse_usage(p, key, ordinal, timestamp),
            Some("task_complete") => self.finish(p, timestamp, "completed"),
            Some("turn_aborted") => self.finish(p, timestamp, "aborted"),
            Some(x) => self.count(format!("event_msg:{}", safe_label(x))),
            None => self.count("event_msg:unknown".into()),
        }
    }
    fn parse_usage(
        &mut self,
        p: &serde_json::Map<String, Value>,
        key: String,
        ordinal: Option<i64>,
        timestamp: Option<String>,
    ) {
        let Some(id) = self.current_turn_id.clone() else {
            self.diag(
                "missing_turn",
                "warning",
                ordinal,
                "token_count 缺少当前 turn。",
            );
            return;
        };
        if self.unavailable_usage_turns.contains(&id) {
            self.diag(
                "usage_unavailable",
                "info",
                ordinal,
                "当前 turn 已有异常 token 快照，保持 token 指标 unavailable。",
            );
            return;
        }
        let info = p.get("info").and_then(Value::as_object);
        let cumulative_value = info.and_then(|x| x.get("total_token_usage"));
        let reported_value = info.and_then(|x| x.get("last_token_usage"));
        let cumulative = cumulative_value.and_then(parse_usage);
        let reported = reported_value.and_then(parse_usage);
        let Some(cumulative) = cumulative else {
            self.diag(
                "invalid_usage",
                "warning",
                ordinal,
                "隔离超出范围、为负或不完整的累计 token 向量。",
            );
            self.mark_usage_unavailable(&id);
            return;
        };
        if reported_value.is_some() && reported.is_none() {
            self.diag(
                "invalid_reported_usage",
                "warning",
                ordinal,
                "忽略超出范围、为负或不完整的本次 token 向量。",
            );
        }
        let reported = match reported {
            Some(value) if !cumulative.monotonic_from(&value) => {
                self.diag(
                    "reported_usage_exceeds_cumulative",
                    "warning",
                    ordinal,
                    "忽略大于累计值的本次 token 向量，并改用累计差值。",
                );
                None
            }
            value => value,
        };
        let calls = self.turn(&id).model_call_count;
        let (delta, confidence) = match &self.last_usage {
            None => {
                let confidence = if reported.as_ref() == Some(&cumulative) {
                    "codex-reported"
                } else {
                    if reported.is_some() {
                        self.diag(
                            "reported_usage_mismatch",
                            "warning",
                            ordinal,
                            "首次 token 快照的本次向量与累计向量不一致，改用累计向量并标记为 derived。",
                        );
                    }
                    "derived"
                };
                (cumulative.clone(), confidence)
            }
            Some(last) if *last == cumulative => {
                self.duplicate_usage += 1;
                return;
            }
            Some(last) if cumulative.monotonic_from(last) => {
                let Some(delta) = cumulative.checked_delta(last) else {
                    self.diag(
                        "usage_delta_out_of_range",
                        "warning",
                        ordinal,
                        "隔离超出安全范围的累计 token 差值。",
                    );
                    self.mark_usage_unavailable(&id);
                    return;
                };
                if reported.as_ref() == Some(&delta) {
                    (reported.unwrap(), "codex-reported")
                } else {
                    (delta, "derived")
                }
            }
            Some(_) if calls == 0 && !cumulative.is_zero() => {
                self.diag(
                    "non_monotonic_usage",
                    "info",
                    ordinal,
                    "接受新 turn 的 token 计数器重置。",
                );
                let confidence = if reported.as_ref() == Some(&cumulative) {
                    "codex-reported"
                } else {
                    if reported.is_some() {
                        self.diag(
                            "reported_usage_mismatch",
                            "warning",
                            ordinal,
                            "重置后首次 token 快照的本次向量与累计向量不一致，改用累计向量并标记为 derived。",
                        );
                    }
                    "derived"
                };
                (cumulative.clone(), confidence)
            }
            Some(_) => {
                self.diag(
                    "non_monotonic_usage",
                    "warning",
                    ordinal,
                    "隔离同一 turn 内回退的累计 token 快照。",
                );
                return;
            }
        };
        if !delta.is_valid_call() {
            self.diag(
                "usage_call_out_of_range",
                "warning",
                ordinal,
                "隔离超出单次模型调用上限或内部不一致的 token 向量。",
            );
            self.mark_usage_unavailable(&id);
            return;
        }
        self.last_usage = Some(cumulative.clone());
        if delta.is_zero() {
            self.duplicate_usage += 1;
            return;
        };
        let session_id = self.session.as_ref().unwrap().session_id.clone();
        let (next_usage, call_count) = {
            let turn = self.turn(&id);
            (
                turn.usage.checked_add(&delta),
                turn.model_call_count.checked_add(1),
            )
        };
        let Some(next_usage) = next_usage else {
            self.diag(
                "usage_aggregate_out_of_range",
                "warning",
                ordinal,
                "隔离会导致 turn token 聚合溢出或超出安全范围的快照。",
            );
            self.mark_usage_unavailable(&id);
            return;
        };
        let Some(call_count) = call_count else {
            self.diag(
                "usage_call_count_overflow",
                "warning",
                ordinal,
                "隔离会导致模型调用计数溢出的快照。",
            );
            self.mark_usage_unavailable(&id);
            return;
        };
        let (model, effort, provider) = {
            let t = self.turn(&id);
            t.usage = next_usage;
            t.model_call_count = call_count;
            t.usage_confidence =
                if t.usage_confidence == "unavailable" || t.usage_confidence == confidence {
                    confidence.into()
                } else {
                    "derived".into()
                };
            (t.model.clone(), t.effort.clone(), t.provider.clone())
        };
        self.model_calls.push(ModelCall {
            event_key: key,
            session_id,
            turn_id: id,
            ordinal,
            occurred_at: timestamp,
            model,
            effort,
            provider,
            usage: delta,
            cumulative_usage: cumulative,
            usage_confidence: confidence.into(),
        });
        self.dirty_turns
            .insert(self.current_turn_id.clone().unwrap_or_default());
    }
    fn finish(
        &mut self,
        p: &serde_json::Map<String, Value>,
        timestamp: Option<String>,
        status: &str,
    ) {
        let Some(id) = val_str(p, "turn_id").or_else(|| self.current_turn_id.clone()) else {
            return;
        };
        let t = self.turn(&id);
        t.started_at = val_str(p, "started_at").or(t.started_at.clone());
        t.completed_at = val_str(p, "completed_at")
            .or(timestamp)
            .or(t.completed_at.clone());
        t.duration_ms = p
            .get("duration_ms")
            .and_then(Value::as_i64)
            .or(t.duration_ms);
        if status == "completed" {
            t.first_visible_output_ms = p
                .get("time_to_first_token_ms")
                .and_then(Value::as_i64)
                .or(t.first_visible_output_ms);
            t.status = if p.get("error").is_some() {
                "failed"
            } else {
                "completed"
            }
            .into()
        } else {
            t.status = status.into()
        }
        self.current_turn_id = Some(id);
        if let Some(id) = &self.current_turn_id {
            self.dirty_turns.insert(id.clone());
        }
    }
    fn mark_usage_unavailable(&mut self, id: &str) {
        let turn = self.turn(id);
        turn.usage = TokenUsage::default();
        turn.model_call_count = 0;
        turn.usage_confidence = "unavailable".into();
        self.unavailable_usage_turns.insert(id.into());
        self.dirty_turns.insert(id.into());
    }
    fn turn(&mut self, id: &str) -> &mut Turn {
        let s = self.session.as_ref().expect("session checked");
        self.turns.entry(id.into()).or_insert_with(|| Turn {
            turn_id: id.into(),
            session_id: s.session_id.clone(),
            model: None,
            effort: None,
            provider: s.model_provider.clone(),
            cwd: s.cwd.clone(),
            started_at: None,
            completed_at: None,
            duration_ms: None,
            first_visible_output_ms: None,
            status: "unknown".into(),
            usage: TokenUsage::default(),
            model_call_count: 0,
            usage_confidence: "unavailable".into(),
        })
    }
    fn event_key(
        &self,
        typ: &str,
        p: &serde_json::Map<String, Value>,
        ordinal: Option<i64>,
        timestamp: Option<&str>,
        offset: Option<u64>,
    ) -> String {
        let sid = self
            .session
            .as_ref()
            .map(|x| x.session_id.as_str())
            .or_else(|| p.get("session_id").and_then(Value::as_str))
            .unwrap_or("unknown-session");
        let pos = ordinal
            .map(|x| format!("ordinal-{x}"))
            .or_else(|| offset.map(|x| format!("offset-{x}")))
            .or_else(|| {
                p.get("id")
                    .or_else(|| p.get("call_id"))
                    .or_else(|| p.get("turn_id"))
                    .and_then(Value::as_str)
                    .map(|x| format!("id-{x}"))
            })
            .unwrap_or_else(|| format!("timestamp-{}", timestamp.unwrap_or("unknown")));
        format!(
            "{sid}:{pos}:{typ}:{}",
            safe_label(p.get("type").and_then(Value::as_str).unwrap_or("none"))
        )
    }
    fn diag(&mut self, code: &str, severity: &str, ordinal: Option<i64>, message: &str) {
        self.diagnostics.push(Diagnostic {
            code: code.into(),
            severity: severity.into(),
            ordinal,
            message: message.into(),
        })
    }
    fn count(&mut self, key: String) {
        *self.unhandled.entry(key).or_default() += 1
    }
}

fn val_str(p: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    p.get(key).and_then(Value::as_str).map(ToOwned::to_owned)
}
fn parse_usage(v: &Value) -> Option<TokenUsage> {
    let o = v.as_object()?;
    let n = |k| o.get(k).and_then(Value::as_i64).filter(|x| *x >= 0);
    let usage = TokenUsage {
        input_tokens: n("input_tokens")?,
        cached_input_tokens: n("cached_input_tokens")?,
        cache_write_input_tokens: n("cache_write_input_tokens")?,
        output_tokens: n("output_tokens")?,
        reasoning_output_tokens: n("reasoning_output_tokens")?,
        total_tokens: n("total_tokens")?,
    };
    (usage.is_valid() && usage.is_consistent()).then_some(usage)
}
fn message_bytes(v: Option<&Value>) -> Option<i64> {
    let mut total = 0_i64;
    let mut found = false;
    for p in v?.as_array()? {
        if let Some(s) = p.get("text").and_then(Value::as_str) {
            found = true;
            total += s.len() as i64
        }
    }
    found.then_some(total)
}
fn safe_label(v: &str) -> String {
    if !v.is_empty()
        && v.len() <= 64
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        v.into()
    } else {
        "unknown".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(typ: &str, p: Value, ordinal: i64) -> String {
        serde_json::json!({"timestamp":"2026-08-27T00:00:00Z","ordinal":ordinal,"type":typ,"payload":p}).to_string()
    }
    fn usage(total: i64) -> Value {
        serde_json::json!({"input_tokens":total,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":total})
    }
    #[test]
    fn deduplicates_and_handles_reset() {
        let mut n = RolloutNormalizer::default();
        n.parse_line(
            &event("session_meta", serde_json::json!({"session_id":"s"}), 0),
            None,
        );
        n.parse_line(
            &event(
                "event_msg",
                serde_json::json!({"type":"task_started","turn_id":"a"}),
                1,
            ),
            None,
        );
        n.parse_line(&event("event_msg",serde_json::json!({"type":"token_count","info":{"total_token_usage":usage(10),"last_token_usage":usage(10)}}),2),None);
        n.parse_line(
            &event(
                "event_msg",
                serde_json::json!({"type":"task_started","turn_id":"b"}),
                3,
            ),
            None,
        );
        n.parse_line(&event("event_msg",serde_json::json!({"type":"token_count","info":{"total_token_usage":usage(3),"last_token_usage":usage(3)}}),4),None);
        let s = n.snapshot();
        assert_eq!(s.model_calls.len(), 2);
        assert_eq!(
            s.turns
                .iter()
                .find(|t| t.turn_id == "b")
                .unwrap()
                .usage
                .total_tokens,
            3
        );
    }
    #[test]
    fn retains_only_message_length() {
        let mut n = RolloutNormalizer::default();
        n.parse_line(
            &event("session_meta", serde_json::json!({"session_id":"s"}), 0),
            None,
        );
        n.parse_line(
            &event(
                "response_item",
                serde_json::json!({"type":"message","content":[{"text":"secret"}]}),
                1,
            ),
            None,
        );
        let s = serde_json::to_string(&n.snapshot()).unwrap();
        assert!(!s.contains("secret"));
        assert!(s.contains("contentUtf8Bytes"));
    }

    #[test]
    fn resumes_a_partial_turn_without_replaying_previous_usage() {
        let mut first = RolloutNormalizer::default();
        first.parse_line(
            &event("session_meta", serde_json::json!({"session_id":"s"}), 0),
            Some(0),
        );
        first.parse_line(
            &event(
                "event_msg",
                serde_json::json!({"type":"task_started","turn_id":"t"}),
                1,
            ),
            Some(10),
        );
        first.parse_line(&event("event_msg",serde_json::json!({"type":"token_count","info":{"total_token_usage":usage(10),"last_token_usage":usage(10)}}),2),Some(20));
        let state = serde_json::from_str::<ResumeState>(
            &serde_json::to_string(&first.resume_state()).unwrap(),
        )
        .unwrap();

        let mut resumed = RolloutNormalizer::from_resume_state(state);
        resumed.parse_line(&event("event_msg",serde_json::json!({"type":"token_count","info":{"total_token_usage":usage(16),"last_token_usage":usage(6)}}),3),Some(30));
        let batch = resumed.incremental_snapshot();
        assert_eq!(batch.model_calls.len(), 1);
        assert_eq!(batch.model_calls[0].usage.total_tokens, 6);
        assert_eq!(batch.turns[0].usage.total_tokens, 16);
        assert_eq!(
            resumed
                .resume_state()
                .last_cumulative_usage
                .unwrap()
                .total_tokens,
            16
        );
    }

    #[test]
    fn token_usage_bounds_prevent_wraparound() {
        let maximum = TokenUsage {
            input_tokens: TokenUsage::MAX_FIELD_VALUE,
            cached_input_tokens: TokenUsage::MAX_FIELD_VALUE,
            cache_write_input_tokens: TokenUsage::MAX_FIELD_VALUE,
            output_tokens: TokenUsage::MAX_FIELD_VALUE,
            reasoning_output_tokens: TokenUsage::MAX_FIELD_VALUE,
            total_tokens: TokenUsage::MAX_FIELD_VALUE,
        };
        let one = TokenUsage {
            input_tokens: 1,
            cached_input_tokens: 1,
            cache_write_input_tokens: 1,
            output_tokens: 1,
            reasoning_output_tokens: 1,
            total_tokens: 1,
        };
        assert!(maximum.checked_add(&one).is_none());
        assert!(one.checked_delta(&maximum).is_none());
    }

    #[test]
    fn invalid_or_inconsistent_usage_stays_unavailable_or_derived() {
        let mut n = RolloutNormalizer::default();
        n.parse_line(
            &event("session_meta", serde_json::json!({"session_id":"s"}), 0),
            None,
        );
        n.parse_line(
            &event(
                "event_msg",
                serde_json::json!({"type":"task_started","turn_id":"t"}),
                1,
            ),
            None,
        );
        n.parse_line(
            &event(
                "event_msg",
                serde_json::json!({"type":"token_count","info":{"total_token_usage":usage(10),"last_token_usage":usage(10)}}),
                2,
            ),
            None,
        );
        n.parse_line(
            &event(
                "event_msg",
                serde_json::json!({"type":"token_count","info":{"total_token_usage":usage(15),"last_token_usage":usage(99)}}),
                3,
            ),
            None,
        );
        assert_eq!(n.snapshot().model_calls[1].usage.total_tokens, 5);
        assert_eq!(n.snapshot().model_calls[1].usage_confidence, "derived");

        let too_large = TokenUsage::MAX_FIELD_VALUE + 1;
        n.parse_line(
            &event(
                "event_msg",
                serde_json::json!({"type":"token_count","info":{"total_token_usage":{"input_tokens":too_large,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":too_large}}}),
                4,
            ),
            None,
        );
        let snapshot = n.snapshot();
        assert_eq!(snapshot.turns[0].usage_confidence, "unavailable");
        assert!(
            snapshot
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "invalid_usage")
        );
    }

    #[test]
    fn first_reported_usage_cannot_understate_cumulative_usage() {
        let mut n = RolloutNormalizer::default();
        n.parse_line(
            &event("session_meta", serde_json::json!({"session_id":"s"}), 0),
            None,
        );
        n.parse_line(
            &event(
                "event_msg",
                serde_json::json!({"type":"task_started","turn_id":"t"}),
                1,
            ),
            None,
        );
        n.parse_line(
            &event(
                "event_msg",
                serde_json::json!({"type":"token_count","info":{"total_token_usage":usage(100),"last_token_usage":usage(1)}}),
                2,
            ),
            None,
        );

        let snapshot = n.snapshot();
        assert_eq!(snapshot.model_calls[0].usage.total_tokens, 100);
        assert_eq!(snapshot.model_calls[0].usage_confidence, "derived");
        assert!(
            snapshot
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "reported_usage_mismatch")
        );
    }

    #[test]
    fn quarantines_implausibly_large_single_call() {
        let mut n = RolloutNormalizer::default();
        n.parse_line(
            &event("session_meta", serde_json::json!({"session_id":"s"}), 0),
            None,
        );
        n.parse_line(
            &event(
                "event_msg",
                serde_json::json!({"type":"task_started","turn_id":"t"}),
                1,
            ),
            None,
        );
        let excessive = TokenUsage::MAX_CALL_FIELD_VALUE + 1;
        n.parse_line(
            &event(
                "event_msg",
                serde_json::json!({"type":"token_count","info":{"total_token_usage":usage(excessive),"last_token_usage":usage(excessive)}}),
                2,
            ),
            None,
        );

        let snapshot = n.snapshot();
        assert!(snapshot.model_calls.is_empty());
        assert_eq!(snapshot.turns[0].usage_confidence, "unavailable");
        assert!(
            snapshot
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "usage_call_out_of_range")
        );
    }
}
