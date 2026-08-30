use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const PRICING_SOURCE_NAME: &str = "Models.dev";
const PRICING_SOURCE_URL: &str = "https://models.dev/api.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelPrice {
    pub model: String,
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: f64,
    #[serde(default = "default_multiplier")]
    pub multiplier: f64,
}

impl ModelPrice {
    fn normalized(mut self) -> Self {
        self.model = self.model.trim().to_owned();
        self
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingSyncResult {
    pub source: &'static str,
    pub source_url: &'static str,
    pub prices: Vec<ModelPrice>,
    pub matched_models: Vec<String>,
    pub unmatched_models: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ModelsDevProvider {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    models: BTreeMap<String, ModelsDevModel>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelsDevModel {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    cost: ModelsDevCost,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ModelsDevCost {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
}

#[derive(Debug, Clone)]
struct CatalogEntry {
    provider_id: String,
    provider_name: String,
    model: ModelsDevModel,
}

pub fn validate_model_prices(prices: &[ModelPrice]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for price in prices {
        let model = price.model.trim();
        if model.is_empty() || model.len() > 256 {
            bail!("model price names must contain 1-256 characters");
        }
        if !seen.insert(model.to_ascii_lowercase()) {
            bail!("model price names must be unique");
        }
        for (name, value) in [
            ("input", price.input),
            ("output", price.output),
            ("cache read", price.cache_read),
            ("cache write", price.cache_write),
            ("multiplier", price.multiplier),
        ] {
            if !value.is_finite() || value < 0.0 {
                bail!("{name} price must be a finite non-negative number");
            }
        }
    }
    Ok(())
}

pub fn normalized_model_prices(prices: Vec<ModelPrice>) -> Result<Vec<ModelPrice>> {
    let mut prices = prices
        .into_iter()
        .map(ModelPrice::normalized)
        .collect::<Vec<_>>();
    validate_model_prices(&prices)?;
    prices.sort_by(|left, right| left.model.cmp(&right.model));
    Ok(prices)
}

pub fn price_index(prices: &[ModelPrice]) -> BTreeMap<String, ModelPrice> {
    prices
        .iter()
        .cloned()
        .map(|price| (price.model.trim().to_ascii_lowercase(), price))
        .collect()
}

pub fn calculate_cost(
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    price: &ModelPrice,
) -> f64 {
    let input_tokens = input_tokens.max(0);
    let output_tokens = output_tokens.max(0);
    let cache_read_tokens = cache_read_tokens.max(0);
    let cache_write_tokens = cache_write_tokens.max(0);
    let regular_input_tokens = input_tokens
        .saturating_sub(cache_read_tokens)
        .saturating_sub(cache_write_tokens);
    let unscaled = regular_input_tokens as f64 * price.input
        + cache_read_tokens as f64 * price.cache_read
        + cache_write_tokens as f64 * price.cache_write
        + output_tokens as f64 * price.output;
    unscaled / 1_000_000.0 * price.multiplier
}

pub async fn sync_model_prices(
    client: &reqwest::Client,
    used_models: &[String],
    existing_prices: &[ModelPrice],
) -> Result<PricingSyncResult> {
    let catalog = client
        .get(PRICING_SOURCE_URL)
        .header("accept", "application/json")
        .send()
        .await
        .context("failed to fetch Models.dev pricing catalog")?
        .error_for_status()
        .context("Models.dev pricing catalog returned an error")?
        .json::<BTreeMap<String, ModelsDevProvider>>()
        .await
        .context("failed to decode Models.dev pricing catalog")?;
    let entries = flatten_catalog(catalog);
    let existing = price_index(existing_prices);
    let mut merged = existing.clone();
    let mut matched_models = Vec::new();
    let mut unmatched_models = Vec::new();
    let unique_models = used_models
        .iter()
        .map(|model| model.trim())
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();

    for model in unique_models {
        let Some(entry) = best_catalog_match(&model, &entries) else {
            unmatched_models.push(model);
            continue;
        };
        let (Some(input), Some(output)) = (entry.model.cost.input, entry.model.cost.output) else {
            unmatched_models.push(model);
            continue;
        };
        if [input, output]
            .into_iter()
            .chain(
                [entry.model.cost.cache_read, entry.model.cost.cache_write]
                    .into_iter()
                    .flatten(),
            )
            .any(|value| !value.is_finite() || value < 0.0)
        {
            unmatched_models.push(model);
            continue;
        }
        let multiplier = existing
            .get(&model.to_ascii_lowercase())
            .map(|price| price.multiplier)
            .unwrap_or_else(default_multiplier);
        merged.insert(
            model.to_ascii_lowercase(),
            ModelPrice {
                model: model.clone(),
                input,
                output,
                cache_read: entry.model.cost.cache_read.unwrap_or(0.0),
                cache_write: entry.model.cost.cache_write.unwrap_or(0.0),
                multiplier,
            },
        );
        matched_models.push(model);
    }

    let prices = normalized_model_prices(merged.into_values().collect())?;
    Ok(PricingSyncResult {
        source: PRICING_SOURCE_NAME,
        source_url: PRICING_SOURCE_URL,
        prices,
        matched_models,
        unmatched_models,
    })
}

fn flatten_catalog(catalog: BTreeMap<String, ModelsDevProvider>) -> Vec<CatalogEntry> {
    let mut entries = Vec::new();
    for (provider_key, provider) in catalog {
        let provider_id = non_empty(&provider.id).unwrap_or(&provider_key).to_owned();
        let provider_name = non_empty(&provider.name).unwrap_or(&provider_id).to_owned();
        for (model_key, mut model) in provider.models {
            if model.id.trim().is_empty() {
                model.id = model_key;
            }
            if model.id.trim().is_empty() && model.name.trim().is_empty() {
                continue;
            }
            entries.push(CatalogEntry {
                provider_id: provider_id.clone(),
                provider_name: provider_name.clone(),
                model,
            });
        }
    }
    entries
}

fn best_catalog_match<'a>(model: &str, entries: &'a [CatalogEntry]) -> Option<&'a CatalogEntry> {
    entries
        .iter()
        .filter_map(|entry| match_score(model, entry).map(|score| (score, entry)))
        .max_by(|(left_score, left), (right_score, right)| {
            left_score
                .cmp(right_score)
                .then_with(|| provider_rank(model, right).cmp(&provider_rank(model, left)))
                .then_with(|| is_deprecated(&right.model).cmp(&is_deprecated(&left.model)))
                .then_with(|| right.provider_id.cmp(&left.provider_id))
        })
        .map(|(_, entry)| entry)
}

fn match_score(requested: &str, entry: &CatalogEntry) -> Option<i32> {
    let requested = requested.trim();
    let suffix = requested.rsplit('/').next().unwrap_or(requested);
    let id = entry.model.id.trim();
    let name = entry.model.name.trim();
    if id.eq_ignore_ascii_case(suffix) {
        return Some(100);
    }
    if name.eq_ignore_ascii_case(suffix) {
        return Some(98);
    }
    if id.eq_ignore_ascii_case(requested) {
        return Some(96);
    }
    if name.eq_ignore_ascii_case(requested) {
        return Some(94);
    }
    let requested = normalized_model_key(suffix);
    if requested.is_empty() {
        return None;
    }
    if normalized_model_key(id) == requested {
        Some(90)
    } else if normalized_model_key(name) == requested {
        Some(88)
    } else {
        None
    }
}

fn provider_rank(model: &str, entry: &CatalogEntry) -> i32 {
    let expected =
        if model.starts_with("gpt-") || model.starts_with("o1") || model.starts_with("o3") {
            "openai"
        } else if model.starts_with("claude") {
            "anthropic"
        } else if model.starts_with("gemini") {
            "google"
        } else {
            ""
        };
    if expected.is_empty() {
        return 1;
    }
    let provider = format!("{} {}", entry.provider_id, entry.provider_name).to_ascii_lowercase();
    if provider.contains(expected) { 0 } else { 2 }
}

fn normalized_model_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_deprecated(model: &ModelsDevModel) -> bool {
    model.status.trim().eq_ignore_ascii_case("deprecated")
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

const fn default_multiplier() -> f64 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_uses_independent_cache_segments_and_multiplier() {
        let price = ModelPrice {
            model: "gpt-test".into(),
            input: 2.0,
            output: 10.0,
            cache_read: 0.5,
            cache_write: 1.5,
            multiplier: 2.0,
        };
        let cost = calculate_cost(1_000_000, 100_000, 400_000, 100_000, &price);
        assert!((cost - 4.7).abs() < 0.000_001);
    }

    #[test]
    fn rejects_duplicate_prices() {
        let price = ModelPrice {
            model: "gpt-test".into(),
            input: 1.0,
            output: 2.0,
            cache_read: 0.0,
            cache_write: 0.0,
            multiplier: 1.0,
        };
        assert!(validate_model_prices(&[price.clone(), price]).is_err());
    }
}
