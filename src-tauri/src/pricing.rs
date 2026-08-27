//! Versioned API-equivalent pricing. These values are estimates, never bills.

use codex_core::TokenUsage;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub catalog_version: String,
    pub currency: String,
    pub effective_from: String,
    pub usage_label: String,
    pub disclaimer: String,
    pub rules: Vec<CatalogRule>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRule {
    pub provider: String,
    pub model_pattern: String,
    pub source_url: String,
    pub rates_usd_per_million: Rates,
    pub long_context: Option<LongContext>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rates {
    pub input: f64,
    pub cached_input: f64,
    pub cache_write_input: f64,
    pub output: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LongContext {
    pub input_token_threshold_exclusive: i64,
    pub input_multiplier: f64,
    pub output_multiplier: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicRule {
    pub id: String,
    pub provider: String,
    pub model_pattern: String,
    pub effective_from: String,
    pub currency: String,
    pub input_per_million: f64,
    pub cached_input_per_million: f64,
    pub cache_write_input_per_million: f64,
    pub output_per_million: f64,
    pub source_url: Option<String>,
    pub source_label: String,
    pub is_user_override: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    pub total_usd: f64,
    pub plain_input_tokens: i64,
    pub long_context_applied: bool,
    pub catalog_version: String,
}

pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../resources/pricing/openai-2026-08-27.json"
        ))
        .expect("内置价格目录必须是有效 JSON")
    })
}

pub fn public_rules() -> Vec<PublicRule> {
    let catalog = catalog();
    catalog
        .rules
        .iter()
        .map(|rule| PublicRule {
            id: format!(
                "{}:{}:{}",
                catalog.catalog_version, rule.provider, rule.model_pattern
            ),
            provider: rule.provider.clone(),
            model_pattern: rule.model_pattern.clone(),
            effective_from: catalog.effective_from.clone(),
            currency: catalog.currency.clone(),
            input_per_million: rule.rates_usd_per_million.input,
            cached_input_per_million: rule.rates_usd_per_million.cached_input,
            cache_write_input_per_million: rule.rates_usd_per_million.cache_write_input,
            output_per_million: rule.rates_usd_per_million.output,
            source_url: Some(rule.source_url.clone()),
            source_label: format!("{}；{}", catalog.usage_label, catalog.disclaimer),
            is_user_override: false,
        })
        .collect()
}

/// Returns a canonical model label only for an exact, public catalog entry.
/// OTel metadata uses this positive allowlist so an arbitrary attribute value
/// cannot be persisted under the trusted `model` field.
pub fn known_model(model: &str) -> Option<&'static str> {
    catalog()
        .rules
        .iter()
        .find(|rule| model == rule.model_pattern)
        .map(|rule| rule.model_pattern.as_str())
}

pub fn estimate(model: Option<&str>, usage: &TokenUsage) -> Option<Estimate> {
    let model = model?;
    let rule = catalog().rules.iter().find(|rule| {
        model == rule.model_pattern
            || model
                .strip_prefix(&rule.model_pattern)
                .is_some_and(|suffix| suffix.starts_with('-'))
    })?;
    let plain_input = usage
        .input_tokens
        .checked_sub(usage.cached_input_tokens)?
        .checked_sub(usage.cache_write_input_tokens)?;
    if plain_input < 0
        || [
            usage.input_tokens,
            usage.cached_input_tokens,
            usage.cache_write_input_tokens,
            usage.output_tokens,
        ]
        .into_iter()
        .any(|tokens| tokens < 0)
    {
        return None;
    }
    let long_context_applied = rule.long_context.as_ref().is_some_and(|long| {
        usage.input_tokens > long.input_token_threshold_exclusive
            && valid_multiplier(long.input_multiplier)
            && valid_multiplier(long.output_multiplier)
    });
    let input_multiplier = if long_context_applied {
        rule.long_context.as_ref()?.input_multiplier
    } else {
        1.0
    };
    let output_multiplier = if long_context_applied {
        rule.long_context.as_ref()?.output_multiplier
    } else {
        1.0
    };
    let rates = &rule.rates_usd_per_million;
    if ![
        rates.input,
        rates.cached_input,
        rates.cache_write_input,
        rates.output,
    ]
    .into_iter()
    .all(|rate| rate.is_finite() && rate >= 0.0)
    {
        return None;
    }
    let total_usd = (plain_input as f64 * rates.input * input_multiplier
        + usage.cached_input_tokens as f64 * rates.cached_input * input_multiplier
        + usage.cache_write_input_tokens as f64 * rates.cache_write_input * input_multiplier
        + usage.output_tokens as f64 * rates.output * output_multiplier)
        / 1_000_000.0;
    Some(Estimate {
        total_usd,
        plain_input_tokens: plain_input,
        long_context_applied,
        catalog_version: catalog().catalog_version.clone(),
    })
}

fn valid_multiplier(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_cache_write_and_does_not_double_charge_reasoning() {
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            cached_input_tokens: 200_000,
            cache_write_input_tokens: 100_000,
            output_tokens: 100_000,
            reasoning_output_tokens: 90_000,
            total_tokens: 1_100_000,
        };
        let result = estimate(Some("gpt-5.6-sol"), &usage).unwrap();
        // The request is above 272k, so all input buckets are 2x and output is 1.5x.
        assert!((result.total_usd - 9.76).abs() < 1e-9);
        let mut changed_reasoning = usage.clone();
        changed_reasoning.reasoning_output_tokens = 0;
        assert_eq!(
            result.total_usd,
            estimate(Some("gpt-5.6-sol"), &changed_reasoning)
                .unwrap()
                .total_usd
        );
    }

    #[test]
    fn rejects_overlapping_input_buckets_and_unknown_model() {
        let invalid = TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 8,
            cache_write_input_tokens: 4,
            ..Default::default()
        };
        assert!(estimate(Some("gpt-5.6-sol"), &invalid).is_none());
        assert!(estimate(Some("relay-model"), &TokenUsage::default()).is_none());
    }

    #[test]
    fn exposes_the_source_for_each_model_rule() {
        let rules = public_rules();
        assert_eq!(rules.len(), 3);
        for rule in rules {
            let source = rule.source_url.expect("bundled rules require a source");
            assert!(source.contains(&rule.model_pattern));
        }
    }
}
