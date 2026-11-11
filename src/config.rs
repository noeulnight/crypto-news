use std::env;

pub struct Telegram {
    pub token: String,
    pub chat_id: String,
}

pub struct Config {
    pub port: u16,
    pub db_path: String,
    pub web_dir: String,
    pub anthropic_key: Option<String>,
    pub anthropic_base: String,
    /// Used for both the hourly judgment and news tagging.
    pub claude_model: String,
    pub telegram: Option<Telegram>,
    pub whale_alert_key: Option<String>,
}

fn var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            port: var("PORT").and_then(|p| p.parse().ok()).unwrap_or(8787),
            db_path: var("DB_PATH").unwrap_or_else(|| "data/crypto-news.db".into()),
            web_dir: var("WEB_DIR").unwrap_or_else(|| "web/build".into()),
            anthropic_key: var("ANTHROPIC_API_KEY"),
            anthropic_base: var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|| "https://api.anthropic.com".into()),
            claude_model: var("CLAUDE_MODEL").unwrap_or_else(|| "claude-opus-5-5".into()),
            telegram: match (var("TELEGRAM_BOT_TOKEN"), var("TELEGRAM_CHAT_ID")) {
                (Some(token), Some(chat_id)) => Some(Telegram { token, chat_id }),
                _ => None,
            },
            whale_alert_key: var("WHALE_ALERT_API_KEY"),
        }
    }
}
