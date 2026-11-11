//! Breaking news (Coinness feed) and optional Claude tagging.

use anyhow::{Context, Result};
use chrono::DateTime;
use serde_json::{Value, json};

use crate::app::App;
use crate::db::RawNews;
use crate::model::{DIRECTIONS, NewsTag};
use crate::notify::{self, Draft};
use crate::util::{HOUR, now};

// The feed rejects limits above 40.
const FEED: &str = "https://api.coinness.com/feed/v1/breaking-news?limit=40";
const TAG_ASSETS: [&str; 6] = ["btc", "eth", "alt", "macro", "regulation", "other"];

const TAG_SYSTEM: &str = "너는 암호화폐 속보를 분류한다. 입력은 뉴스 배열(JSON)이다. 각 항목에 대해 \
id를 그대로 돌려주고 다음을 판정한다.\n\
- asset: 가격 영향이 가장 직접적인 대상. btc, eth, alt(그 외 코인), macro(금리·달러·증시 등 거시), \
regulation(규제·정책·소송), other 중 하나.\n\
- direction: 해당 대상의 단기 가격에 대한 방향. bullish, bearish, neutral 중 하나. 판단 근거가 약하면 neutral.\n\
- importance: 1(일상적 공지·소형 이슈), 2(시장 참여자가 알아야 할 뉴스), 3(시장 전체를 움직일 수 있는 뉴스).\n\
입력의 모든 항목을 빠짐없이 한 번씩 분류한다.";

fn text(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(Into::into)
}

fn truncate(s: &str, chars: usize) -> String {
    match s.char_indices().nth(chars) {
        Some((idx, _)) => format!("{}…", &s[..idx]),
        None => s.to_owned(),
    }
}

pub async fn poll(app: App) -> Result<()> {
    let v = app.http.json(FEED).await?;
    let rows = v.as_array().context("coinness: feed is not an array")?;
    // The first run only seeds history; alerting on hours-old headlines would be noise.
    let seeded = !app.store.news(1, None)?.is_empty();
    for row in rows.iter().rev() {
        let (Some(id), Some(ts), Some(title)) = (
            row["id"].as_i64(),
            row["publishAt"]
                .as_str()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|d| d.timestamp()),
            row["title"].as_str(),
        ) else {
            continue;
        };
        let item = RawNews {
            id,
            ts,
            title: title.into(),
            content: row["content"].as_str().unwrap_or_default().into(),
            link: text(&row["link"]),
            important: row["isImportant"].as_bool().unwrap_or(false),
            bull: row["bullCount"].as_i64().unwrap_or(0),
            bear: row["bearCount"].as_i64().unwrap_or(0),
        };
        if app.store.upsert_news(&item)? && seeded && item.important {
            notify::emit(
                &app,
                Draft {
                    tier: "realtime",
                    severity: "info",
                    rule: "news_important",
                    asset: None,
                    title: item.title.clone(),
                    body: truncate(&item.content, 300),
                    dedupe: Some(format!("news:{id}")),
                    cooldown: 0,
                },
            )?;
        }
    }
    Ok(())
}

fn tag_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer" },
                        "asset": { "type": "string", "enum": TAG_ASSETS },
                        "direction": { "type": "string", "enum": DIRECTIONS },
                        "importance": { "type": "integer", "enum": [1, 2, 3] }
                    },
                    "required": ["id", "asset", "direction", "importance"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["items"],
        "additionalProperties": false
    })
}

/// Tags recent untagged headlines in one batched call.
pub async fn tag(app: App) -> Result<()> {
    let Some(claude) = &app.claude else {
        return Ok(());
    };
    let items = app.store.untagged_news(now() - 6 * HOUR, 20)?;
    if items.is_empty() {
        return Ok(());
    }
    let input: Vec<Value> = items
        .iter()
        .map(|n| json!({ "id": n.id, "title": n.title, "content": truncate(&n.content, 400) }))
        .collect();
    // Thinking is always on and counts toward max_tokens, so the cap leaves room for it.
    let reply = claude
        .structured(&app.cfg.claude_model, TAG_SYSTEM, &Value::Array(input).to_string(), tag_schema(), 8000)
        .await?;
    for t in reply.json["items"]
        .as_array()
        .context("claude: items missing")?
    {
        // Enum casing is not guaranteed by structured outputs.
        let (Some(id), Some(asset), Some(direction), Some(importance)) = (
            t["id"]
                .as_i64()
                .filter(|id| items.iter().any(|n| n.id == *id)),
            t["asset"]
                .as_str()
                .map(str::to_lowercase)
                .filter(|a| TAG_ASSETS.contains(&a.as_str())),
            t["direction"]
                .as_str()
                .map(str::to_lowercase)
                .filter(|d| DIRECTIONS.contains(&d.as_str())),
            t["importance"].as_i64().filter(|i| (1..=3).contains(i)),
        ) else {
            continue;
        };
        app.store.set_news_tag(
            id,
            &NewsTag {
                asset,
                direction,
                importance,
            },
        )?;
    }
    Ok(())
}
