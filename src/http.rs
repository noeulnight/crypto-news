use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

// Several of the public endpoints used here reject non-browser user agents.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/130.0 Safari/537.36";

#[derive(Clone)]
pub struct Http(reqwest::Client);

impl Http {
    pub fn new() -> Result<Self> {
        Ok(Self(
            reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .timeout(Duration::from_secs(30))
                .gzip(true)
                .build()?,
        ))
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.0
    }

    async fn get(&self, url: &str) -> Result<reqwest::Response> {
        let res = self
            .0
            .get(url)
            .header("Accept", "*/*")
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !res.status().is_success() {
            bail!("GET {url}: HTTP {}", res.status());
        }
        Ok(res)
    }

    pub async fn json(&self, url: &str) -> Result<Value> {
        self.get(url)
            .await?
            .json()
            .await
            .with_context(|| format!("GET {url}: invalid JSON"))
    }

    pub async fn text(&self, url: &str) -> Result<String> {
        self.get(url)
            .await?
            .text()
            .await
            .with_context(|| format!("GET {url}: body"))
    }
}
