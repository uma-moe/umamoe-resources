//! Asks Claude for the English names Global would ship, in batches, with Global's own names as
//! examples. There is no official Rust SDK, so this calls the Messages API over HTTP.

use super::types::{TranslationExample, TranslationItem, TRANSLATION_MODEL};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use tracing::warn;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2026-10-21";
/// Lets a request a safety classifier declines finish on the fallback model it names.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// Names per request: small enough to answer well within the output limit.
pub(super) const BATCH_SIZE: usize = 80;
const MAX_TOKENS: u32 = 16_000;
const ATTEMPTS: u32 = 3;

const INSTRUCTIONS: &str = r#"You are the English localizer for Umamusume: Pretty Derby's Global release. For each Japanese name you are given, write the English name the Global team would ship.

The examples below are names Global has shipped. Before translating, study how Global localizes:
- Global localizes, it does not translate. Titles are rewritten into short, idiomatic, punchy English that captures the feeling or joke, often dropping literal words or restructuring the phrase. A literal rendering is usually wrong.
- `character` is the English name of the card's character. Global titles often play on that character's personality, catchphrases, role or relationships, and sometimes use the character's name or a nickname (for example the student council vice president as "the Vice Prez"). Use what you know about the character to pick wording that fits them.
- Typography: copy the examples exactly. Keep the brackets, use Title Case for outfit titles, write ☆ and ♪ without spaces around them, and keep punctuation such as ! and ? as in the Japanese.
- Katakana loanwords: work out the intended English word, then choose the most natural English phrasing, which is not always the loanword itself.

Kinds:
- uma_name: a character name. Racehorse characters use the real horse's official registered English name as used by JRA/JBIS, not a phonetic guess. Staff and other non-horse characters follow the forms the examples use for such characters.
- outfit_title: an outfit title in brackets, such as [スペシャルドリーマー] → [Special Dreamer].
- support_title: a support card title in brackets, such as [トレセン学園] → [Tracen Academy]. These are often short phrases or quotes in the character's voice.

For each item, consider the meaning, the character and how the examples handle similar cases, then commit to one answer. Answer every id you are given, with the English name only.

Shipped Global names:
"#;

pub(super) struct Translator {
    client: reqwest::Client,
    api_key: String,
    system: String,
}

#[derive(Deserialize)]
struct Answer {
    translations: Vec<AnswerItem>,
}

#[derive(Deserialize)]
struct AnswerItem {
    id: String,
    en: String,
}

impl Translator {
    pub(super) fn new(api_key: String, examples: &[TranslationExample]) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()?;
        let system = format!("{INSTRUCTIONS}{}", serde_json::to_string(examples)?);
        Ok(Self {
            client,
            api_key,
            system,
        })
    }

    /// The English name of each item by its id. An item the model leaves out is missing.
    pub(super) async fn translate(
        &self,
        items: &[TranslationItem],
    ) -> Result<BTreeMap<String, String>> {
        let body = json!({
            "model": TRANSLATION_MODEL,
            "max_tokens": MAX_TOKENS,
            // The instructions and examples are the same for every batch, so later ones read them
            // from the cache.
            "system": [{"type": "text", "text": self.system, "cache_control": {"type": "ephemeral"}}],
            "messages": [{"role": "user", "content": serde_json::to_string(items)?}],
            "output_config": {"format": {"type": "json_schema", "schema": answerSchema()}},
            "fallbacks": "default",
        });

        let response = self.send(&body).await?;
        let stop_reason = response
            .get("stop_reason")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if stop_reason == "refusal" || stop_reason == "max_tokens" {
            bail!("translation stopped early: {stop_reason}");
        }
        let text: String = response
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect();
        let answer: Answer =
            serde_json::from_str(&text).context("translation answer is not the requested JSON")?;

        let mut names = BTreeMap::new();
        for item in answer.translations {
            let en = item.en.trim();
            if !en.is_empty() && items.iter().any(|asked| asked.id == item.id) {
                names.insert(item.id, en.to_string());
            }
        }
        Ok(names)
    }

    /// Sends one request, retrying rate limits, overloads and dropped connections.
    async fn send(&self, body: &Value) -> Result<Value> {
        let mut attempt = 1;
        loop {
            let result = self
                .client
                .post(API_URL)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", API_VERSION)
                .header("anthropic-beta", FALLBACK_BETA)
                .json(body)
                .send()
                .await;
            let retryable = match result {
                Ok(response) if response.status().is_success() => {
                    return response
                        .json()
                        .await
                        .context("translation response is not JSON");
                }
                Ok(response) => {
                    let status = response.status();
                    let detail = response.text().await.unwrap_or_default();
                    let retryable = status.as_u16() == 429 || status.is_server_error();
                    if !retryable || attempt == ATTEMPTS {
                        bail!(
                            "translation request failed with {status}: {}",
                            truncate(&detail)
                        );
                    }
                    format!("{status}")
                }
                Err(error) if attempt < ATTEMPTS => format!("{error}"),
                Err(error) => return Err(error).context("translation request failed"),
            };
            warn!(
                attempt,
                reason = retryable,
                "translation request failed; retrying"
            );
            tokio::time::sleep(Duration::from_secs(20 * u64::from(attempt))).await;
            attempt += 1;
        }
    }
}

fn answerSchema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "translations": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {"id": {"type": "string"}, "en": {"type": "string"}},
                    "required": ["id", "en"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["translations"],
        "additionalProperties": false
    })
}

fn truncate(text: &str) -> &str {
    match text.char_indices().nth(500) {
        Some((end, _)) => &text[..end],
        None => text,
    }
}
