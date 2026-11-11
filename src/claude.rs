use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

// Version used by the structured-outputs reference (`output_config.format`).
const API_VERSION: &str = "2026-10-21";

pub struct Claude {
    http: reqwest::Client,
    base: String,
    key: String,
}

pub struct Reply {
    pub json: Value,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

impl Claude {
    pub fn new(http: reqwest::Client, base: String, key: String) -> Self {
        Self { http, base, key }
    }

    /// One Messages API call constrained to `schema`; returns the parsed JSON object.
    pub async fn structured(
        &self,
        model: &str,
        system: &str,
        user: &str,
        schema: Value,
        max_tokens: u32,
    ) -> Result<Reply> {
        let body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "system": system,
            "messages": [{ "role": "user", "content": user }],
            "output_config": { "format": { "type": "json_schema", "schema": schema } },
        });
        let res = self
            .http
            .post(format!("{}/v1/messages", self.base))
            .header("x-api-key", &self.key)
            .header("anthropic-version", API_VERSION)
            .timeout(Duration::from_secs(180))
            .json(&body)
            .send()
            .await
            .context("claude request")?;
        let status = res.status();
        let v: Value = res.json().await.context("claude response body")?;
        if !status.is_success() {
            bail!(
                "claude HTTP {status}: {}",
                v["error"]["message"].as_str().unwrap_or("unknown error")
            );
        }
        // Refusals and truncated output do not follow the schema.
        let stop = v["stop_reason"].as_str().unwrap_or("");
        if stop != "end_turn" {
            bail!("claude stop_reason={stop}");
        }
        let text = v["content"]
            .as_array()
            .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
            .and_then(|b| b["text"].as_str())
            .context("claude response has no text block")?;
        Ok(Reply {
            json: serde_json::from_str(text).context("claude text is not JSON")?,
            input_tokens: v["usage"]["input_tokens"].as_i64().unwrap_or(0),
            output_tokens: v["usage"]["output_tokens"].as_i64().unwrap_or(0),
        })
    }
}
