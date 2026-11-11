use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::model::{Alert, Brief, Event, Judgment, NewsItem, NewsTag, Verdict};

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS points (
    metric TEXT NOT NULL,
    ts INTEGER NOT NULL,
    value REAL NOT NULL,
    PRIMARY KEY (metric, ts)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS news (
    id INTEGER PRIMARY KEY,
    ts INTEGER NOT NULL,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    link TEXT,
    important INTEGER NOT NULL,
    bull INTEGER NOT NULL,
    bear INTEGER NOT NULL,
    tag_asset TEXT,
    tag_direction TEXT,
    tag_importance INTEGER
);
CREATE INDEX IF NOT EXISTS news_ts ON news (ts);
CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    ts INTEGER NOT NULL,
    title TEXT NOT NULL,
    kind TEXT NOT NULL,
    impact TEXT NOT NULL,
    forecast TEXT,
    previous TEXT
);
CREATE INDEX IF NOT EXISTS events_ts ON events (ts);
CREATE TABLE IF NOT EXISTS alerts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,
    tier TEXT NOT NULL,
    severity TEXT NOT NULL,
    rule TEXT NOT NULL,
    asset TEXT,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    dedupe TEXT UNIQUE
);
CREATE INDEX IF NOT EXISTS alerts_rule ON alerts (rule, asset, ts);
CREATE TABLE IF NOT EXISTS judgments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,
    asset TEXT NOT NULL,
    source TEXT NOT NULL,
    model TEXT NOT NULL,
    regime TEXT NOT NULL,
    bias_4h TEXT NOT NULL,
    bias_24h TEXT NOT NULL,
    confidence TEXT NOT NULL,
    summary TEXT NOT NULL,
    drivers TEXT NOT NULL,
    conflicts TEXT NOT NULL,
    invalidation TEXT NOT NULL,
    price REAL NOT NULL,
    ret_4h REAL,
    ret_24h REAL,
    hit_4h INTEGER,
    hit_24h INTEGER,
    input_tokens INTEGER,
    output_tokens INTEGER
);
CREATE INDEX IF NOT EXISTS judgments_asset ON judgments (asset, source, ts);
CREATE TABLE IF NOT EXISTS briefs (
    kind TEXT NOT NULL,
    ts INTEGER NOT NULL,
    body TEXT NOT NULL,
    PRIMARY KEY (kind, ts)
);
";

const UPSERT_POINT: &str = "INSERT INTO points (metric, ts, value) VALUES (?1, ?2, ?3)
     ON CONFLICT (metric, ts) DO UPDATE SET value = excluded.value";

const NEWS_COLS: &str =
    "id, ts, title, content, link, important, bull, bear, tag_asset, tag_direction, tag_importance";
const JUDGMENT_COLS: &str =
    "id, ts, asset, source, model, regime, bias_4h, bias_24h, confidence, summary, drivers,
     conflicts, invalidation, price, ret_4h, ret_24h, hit_4h, hit_24h";

pub struct RawNews {
    pub id: i64,
    pub ts: i64,
    pub title: String,
    pub content: String,
    pub link: Option<String>,
    pub important: bool,
    pub bull: i64,
    pub bear: i64,
}

pub struct NewAlert<'a> {
    pub ts: i64,
    pub tier: &'a str,
    pub severity: &'a str,
    pub rule: &'a str,
    pub asset: Option<&'a str>,
    pub title: &'a str,
    pub body: &'a str,
    pub dedupe: Option<&'a str>,
}

#[derive(Clone, Copy)]
pub enum Horizon {
    H4,
    H24,
}

impl Horizon {
    pub fn secs(self) -> i64 {
        match self {
            Horizon::H4 => 4 * 3600,
            Horizon::H24 => 24 * 3600,
        }
    }
}

#[derive(Clone)]
pub struct Store(Arc<Mutex<Connection>>);

fn news_row(r: &Row) -> rusqlite::Result<NewsItem> {
    let tag = match (
        r.get::<_, Option<String>>(8)?,
        r.get::<_, Option<String>>(9)?,
        r.get::<_, Option<i64>>(10)?,
    ) {
        (Some(asset), Some(direction), Some(importance)) => Some(NewsTag {
            asset,
            direction,
            importance,
        }),
        _ => None,
    };
    Ok(NewsItem {
        id: r.get(0)?,
        ts: r.get(1)?,
        title: r.get(2)?,
        content: r.get(3)?,
        link: r.get(4)?,
        important: r.get(5)?,
        bull: r.get(6)?,
        bear: r.get(7)?,
        tag,
        react_5m_pct: None,
        react_1h_pct: None,
    })
}

fn judgment_row(r: &Row) -> rusqlite::Result<Judgment> {
    let drivers: String = r.get(10)?;
    Ok(Judgment {
        id: r.get(0)?,
        ts: r.get(1)?,
        asset: r.get(2)?,
        source: r.get(3)?,
        model: r.get(4)?,
        regime: r.get(5)?,
        bias_4h: r.get(6)?,
        bias_24h: r.get(7)?,
        confidence: r.get(8)?,
        summary: r.get(9)?,
        drivers: serde_json::from_str(&drivers).unwrap_or_default(),
        conflicts: r.get(11)?,
        invalidation: r.get(12)?,
        price: r.get(13)?,
        ret_4h_pct: r.get(14)?,
        ret_24h_pct: r.get(15)?,
        hit_4h: r.get(16)?,
        hit_24h: r.get(17)?,
    })
}

fn alert_row(r: &Row) -> rusqlite::Result<Alert> {
    Ok(Alert {
        id: r.get(0)?,
        ts: r.get(1)?,
        tier: r.get(2)?,
        severity: r.get(3)?,
        rule: r.get(4)?,
        asset: r.get(5)?,
        title: r.get(6)?,
        body: r.get(7)?,
    })
}

fn pair(r: &Row) -> rusqlite::Result<(i64, f64)> {
    Ok((r.get(0)?, r.get(1)?))
}

impl Store {
    pub fn open(path: &str) -> Result<Self> {
        if let Some(dir) = Path::new(path)
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
        {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        self.0.lock().expect("db mutex poisoned")
    }

    // ---- time series ----

    pub fn put(&self, metric: &str, ts: i64, value: f64) -> Result<()> {
        if value.is_finite() {
            self.db()
                .prepare_cached(UPSERT_POINT)?
                .execute(params![metric, ts, value])?;
        }
        Ok(())
    }

    pub fn put_many(&self, metric: &str, rows: &[(i64, f64)]) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        {
            let mut st = tx.prepare_cached(UPSERT_POINT)?;
            for &(ts, value) in rows {
                if value.is_finite() {
                    st.execute(params![metric, ts, value])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Accumulates into a bucket (used for per-minute liquidation sums).
    pub fn add(&self, metric: &str, ts: i64, value: f64) -> Result<()> {
        if value.is_finite() {
            self.db()
                .prepare_cached(
                    "INSERT INTO points (metric, ts, value) VALUES (?1, ?2, ?3)
                     ON CONFLICT (metric, ts) DO UPDATE SET value = value + excluded.value",
                )?
                .execute(params![metric, ts, value])?;
        }
        Ok(())
    }

    pub fn latest(&self, metric: &str) -> Result<Option<(i64, f64)>> {
        Ok(self
            .db()
            .prepare_cached(
                "SELECT ts, value FROM points WHERE metric = ?1 ORDER BY ts DESC LIMIT 1",
            )?
            .query_row(params![metric], pair)
            .optional()?)
    }

    /// Latest point with `ts - max_age <= point.ts <= ts`.
    pub fn at_or_before(&self, metric: &str, ts: i64, max_age: i64) -> Result<Option<(i64, f64)>> {
        Ok(self
            .db()
            .prepare_cached(
                "SELECT ts, value FROM points WHERE metric = ?1 AND ts <= ?2 AND ts >= ?3 ORDER BY ts DESC LIMIT 1",
            )?
            .query_row(params![metric, ts, ts - max_age], pair)
            .optional()?)
    }

    pub fn range(&self, metric: &str, from: i64, to: i64) -> Result<Vec<(i64, f64)>> {
        let db = self.db();
        let mut st = db.prepare_cached(
            "SELECT ts, value FROM points WHERE metric = ?1 AND ts >= ?2 AND ts <= ?3 ORDER BY ts",
        )?;
        let rows = st
            .query_map(params![metric, from, to], pair)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Last point of every `step`-second bucket, keeping the point's own timestamp.
    pub fn bucket_last(
        &self,
        metric: &str,
        from: i64,
        to: i64,
        step: i64,
    ) -> Result<Vec<(i64, f64)>> {
        let db = self.db();
        // SQLite returns bare columns from the row that holds MAX(ts).
        let mut st = db.prepare_cached(
            "SELECT MAX(ts), value FROM points WHERE metric = ?1 AND ts >= ?2 AND ts <= ?3 GROUP BY ts / ?4 ORDER BY 1",
        )?;
        let rows = st
            .query_map(params![metric, from, to, step], pair)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Sum per `step`-second bucket, keyed by bucket start. Empty buckets are absent.
    pub fn bucket_sum(
        &self,
        metric: &str,
        from: i64,
        to: i64,
        step: i64,
    ) -> Result<Vec<(i64, f64)>> {
        let db = self.db();
        let mut st = db.prepare_cached(
            "SELECT (ts / ?4) * ?4, SUM(value) FROM points WHERE metric = ?1 AND ts >= ?2 AND ts <= ?3
             GROUP BY ts / ?4 ORDER BY 1",
        )?;
        let rows = st
            .query_map(params![metric, from, to, step], pair)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn sum(&self, metric: &str, from: i64, to: i64) -> Result<f64> {
        Ok(self
            .db()
            .prepare_cached("SELECT COALESCE(SUM(value), 0.0) FROM points WHERE metric = ?1 AND ts >= ?2 AND ts <= ?3")?
            .query_row(params![metric, from, to], |r| r.get(0))?)
    }

    // ---- news ----

    /// Returns true when the item is new. Vote counts of known items are refreshed.
    pub fn upsert_news(&self, n: &RawNews) -> Result<bool> {
        let db = self.db();
        let inserted =
            db.prepare_cached(
                "INSERT OR IGNORE INTO news (id, ts, title, content, link, important, bull, bear)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?
            .execute(params![
                n.id,
                n.ts,
                n.title,
                n.content,
                n.link,
                n.important,
                n.bull,
                n.bear
            ])? > 0;
        if !inserted {
            db.prepare_cached("UPDATE news SET bull = ?2, bear = ?3 WHERE id = ?1")?
                .execute(params![n.id, n.bull, n.bear])?;
        }
        Ok(inserted)
    }

    pub fn news(&self, limit: i64, before_id: Option<i64>) -> Result<Vec<NewsItem>> {
        let db = self.db();
        let mut st = db.prepare_cached(&format!(
            "SELECT {NEWS_COLS} FROM news WHERE id < ?1 ORDER BY id DESC LIMIT ?2"
        ))?;
        let rows = st
            .query_map(params![before_id.unwrap_or(i64::MAX), limit], news_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn news_since(&self, since: i64, limit: i64) -> Result<Vec<NewsItem>> {
        let db = self.db();
        let mut st = db.prepare_cached(&format!(
            "SELECT {NEWS_COLS} FROM news WHERE ts >= ?1 ORDER BY ts DESC LIMIT ?2"
        ))?;
        let rows = st
            .query_map(params![since, limit], news_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn untagged_news(&self, since: i64, limit: i64) -> Result<Vec<NewsItem>> {
        let db = self.db();
        let mut st = db.prepare_cached(&format!(
            "SELECT {NEWS_COLS} FROM news WHERE ts >= ?1 AND tag_asset IS NULL ORDER BY ts DESC LIMIT ?2"
        ))?;
        let rows = st
            .query_map(params![since, limit], news_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn set_news_tag(&self, id: i64, tag: &NewsTag) -> Result<()> {
        self.db()
            .prepare_cached("UPDATE news SET tag_asset = ?2, tag_direction = ?3, tag_importance = ?4 WHERE id = ?1")?
            .execute(params![id, tag.asset, tag.direction, tag.importance])?;
        Ok(())
    }

    // ---- events ----

    pub fn upsert_event(&self, e: &Event) -> Result<()> {
        self.db()
            .prepare_cached(
                "INSERT INTO events (id, ts, title, kind, impact, forecast, previous) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (id) DO UPDATE SET ts = excluded.ts, title = excluded.title, impact = excluded.impact,
                     forecast = excluded.forecast, previous = excluded.previous",
            )?
            .execute(params![e.id, e.ts, e.title, e.kind, e.impact, e.forecast, e.previous])?;
        Ok(())
    }

    pub fn events(&self, from: i64, to: i64) -> Result<Vec<Event>> {
        let db = self.db();
        let mut st = db.prepare_cached(
            "SELECT id, ts, title, kind, impact, forecast, previous FROM events WHERE ts >= ?1 AND ts <= ?2 ORDER BY ts",
        )?;
        let rows = st
            .query_map(params![from, to], |r| {
                Ok(Event {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    title: r.get(2)?,
                    kind: r.get(3)?,
                    impact: r.get(4)?,
                    forecast: r.get(5)?,
                    previous: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ---- alerts ----

    /// Returns `None` when `dedupe` already exists.
    pub fn insert_alert(&self, a: &NewAlert) -> Result<Option<Alert>> {
        let db = self.db();
        let n = db
            .prepare_cached(
                "INSERT OR IGNORE INTO alerts (ts, tier, severity, rule, asset, title, body, dedupe)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?
            .execute(params![a.ts, a.tier, a.severity, a.rule, a.asset, a.title, a.body, a.dedupe])?;
        if n == 0 {
            return Ok(None);
        }
        Ok(Some(Alert {
            id: db.last_insert_rowid(),
            ts: a.ts,
            tier: a.tier.into(),
            severity: a.severity.into(),
            rule: a.rule.into(),
            asset: a.asset.map(Into::into),
            title: a.title.into(),
            body: a.body.into(),
        }))
    }

    pub fn last_alert_ts(&self, rule: &str, asset: Option<&str>) -> Result<Option<i64>> {
        Ok(self
            .db()
            .prepare_cached("SELECT MAX(ts) FROM alerts WHERE rule = ?1 AND asset IS ?2")?
            .query_row(params![rule, asset], |r| r.get(0))?)
    }

    pub fn alerts(&self, limit: i64) -> Result<Vec<Alert>> {
        let db = self.db();
        let mut st = db.prepare_cached(
            "SELECT id, ts, tier, severity, rule, asset, title, body FROM alerts ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = st
            .query_map(params![limit], alert_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ---- judgments ----

    pub fn insert_judgment(
        &self,
        ts: i64,
        asset: &str,
        price: f64,
        v: &Verdict,
    ) -> Result<Judgment> {
        let db = self.db();
        db.prepare_cached(
            "INSERT INTO judgments (ts, asset, source, model, regime, bias_4h, bias_24h, confidence, summary, drivers,
                 conflicts, invalidation, price, input_tokens, output_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        )?
        .execute(params![
            ts,
            asset,
            v.source,
            v.model,
            v.regime,
            v.bias_4h,
            v.bias_24h,
            v.confidence,
            v.summary,
            serde_json::to_string(&v.drivers)?,
            v.conflicts,
            v.invalidation,
            price,
            v.tokens.map(|t| t.0),
            v.tokens.map(|t| t.1),
        ])?;
        Ok(Judgment {
            id: db.last_insert_rowid(),
            ts,
            asset: asset.into(),
            source: v.source.into(),
            model: v.model.clone(),
            regime: v.regime.clone(),
            bias_4h: v.bias_4h.clone(),
            bias_24h: v.bias_24h.clone(),
            confidence: v.confidence.clone(),
            summary: v.summary.clone(),
            drivers: v.drivers.clone(),
            conflicts: v.conflicts.clone(),
            invalidation: v.invalidation.clone(),
            price,
            ret_4h_pct: None,
            ret_24h_pct: None,
            hit_4h: None,
            hit_24h: None,
        })
    }

    pub fn judgments(
        &self,
        asset: &str,
        source: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Judgment>> {
        let db = self.db();
        let mut st = db.prepare_cached(&format!(
            "SELECT {JUDGMENT_COLS} FROM judgments WHERE asset = ?1 AND (?2 IS NULL OR source = ?2)
             ORDER BY ts DESC, id DESC LIMIT ?3"
        ))?;
        let rows = st
            .query_map(params![asset, source, limit], judgment_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Judgments whose horizon has elapsed but are not scored yet (only recent ones are retried).
    pub fn unscored(&self, h: Horizon, now: i64) -> Result<Vec<Judgment>> {
        let col = match h {
            Horizon::H4 => "ret_4h",
            Horizon::H24 => "ret_24h",
        };
        let db = self.db();
        let mut st = db.prepare_cached(&format!(
            "SELECT {JUDGMENT_COLS} FROM judgments WHERE {col} IS NULL AND ts <= ?1 AND ts >= ?2 ORDER BY ts"
        ))?;
        let rows = st
            .query_map(
                params![now - h.secs(), now - h.secs() - 3 * 86_400],
                judgment_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn set_outcome(&self, id: i64, h: Horizon, ret_pct: f64, hit: bool) -> Result<()> {
        let sql = match h {
            Horizon::H4 => "UPDATE judgments SET ret_4h = ?2, hit_4h = ?3 WHERE id = ?1",
            Horizon::H24 => "UPDATE judgments SET ret_24h = ?2, hit_24h = ?3 WHERE id = ?1",
        };
        self.db()
            .prepare_cached(sql)?
            .execute(params![id, ret_pct, hit])?;
        Ok(())
    }

    // ---- briefs ----

    pub fn put_brief(&self, kind: &str, ts: i64, body: &str) -> Result<()> {
        self.db()
            .prepare_cached("INSERT OR REPLACE INTO briefs (kind, ts, body) VALUES (?1, ?2, ?3)")?
            .execute(params![kind, ts, body])?;
        Ok(())
    }

    pub fn latest_brief(&self, kind: &str) -> Result<Option<Brief>> {
        Ok(self
            .db()
            .prepare_cached("SELECT ts, body FROM briefs WHERE kind = ?1 ORDER BY ts DESC LIMIT 1")?
            .query_row(params![kind], |r| {
                Ok(Brief {
                    ts: r.get(0)?,
                    body: r.get(1)?,
                })
            })
            .optional()?)
    }
}
