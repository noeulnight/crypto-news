use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Serialize;
use tokio::sync::broadcast;

use crate::claude::Claude;
use crate::config::Config;
use crate::db::Store;
use crate::http::Http;
use crate::model::{Alert, Asset, Judgment};
use crate::signal::snapshot::{self, Snapshot};
use crate::util::now;

const SNAPSHOT_TTL: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub enum BusEvent {
    Price { asset: Asset, ts: i64, price: f64 },
    Alert(Alert),
    Judgment(Judgment),
}

#[derive(Clone, Default, Serialize)]
pub struct CollectorStatus {
    pub name: &'static str,
    pub last_ok_ts: Option<i64>,
    pub last_err: Option<String>,
    pub last_err_ts: Option<i64>,
    /// Expected seconds between runs; `None` for streams.
    pub every_secs: Option<u64>,
}

#[derive(Clone, Default)]
pub struct Status(Arc<Mutex<BTreeMap<&'static str, CollectorStatus>>>);

impl Status {
    pub fn ok(&self, name: &'static str) {
        let mut map = self.0.lock().expect("status mutex poisoned");
        let s = map.entry(name).or_default();
        s.name = name;
        s.last_ok_ts = Some(now());
        s.last_err = None;
        s.last_err_ts = None;
    }

    /// Declares a periodic job, so a reader can tell "late" from "not due yet".
    pub fn expect(&self, name: &'static str, every_secs: u64) {
        let mut map = self.0.lock().expect("status mutex poisoned");
        let s = map.entry(name).or_default();
        s.name = name;
        s.every_secs = Some(every_secs);
    }

    pub fn err(&self, name: &'static str, msg: String) {
        let mut map = self.0.lock().expect("status mutex poisoned");
        let s = map.entry(name).or_default();
        s.name = name;
        s.last_err = Some(msg);
        s.last_err_ts = Some(now());
    }

    pub fn list(&self) -> Vec<CollectorStatus> {
        self.0
            .lock()
            .expect("status mutex poisoned")
            .values()
            .cloned()
            .collect()
    }
}

type SnapshotCache = Arc<Mutex<HashMap<Asset, (Instant, Arc<Snapshot>)>>>;

#[derive(Clone)]
pub struct App {
    pub cfg: Arc<Config>,
    pub store: Store,
    pub http: Http,
    pub status: Status,
    pub bus: broadcast::Sender<BusEvent>,
    pub claude: Option<Arc<Claude>>,
    snapshots: SnapshotCache,
}

impl App {
    pub fn new(cfg: Arc<Config>) -> Result<Self> {
        let http = Http::new()?;
        let claude = cfg.anthropic_key.as_ref().map(|key| {
            Arc::new(Claude::new(
                http.client().clone(),
                cfg.anthropic_base.clone(),
                key.clone(),
            ))
        });
        Ok(Self {
            store: Store::open(&cfg.db_path)?,
            http,
            status: Status::default(),
            bus: broadcast::channel(256).0,
            claude,
            snapshots: Arc::default(),
            cfg,
        })
    }

    pub fn whale_enabled(&self) -> bool {
        self.cfg.whale_alert_key.is_some()
    }

    /// Indicator snapshot, cached briefly because every API poll and the rule loop ask for it.
    pub fn snapshot(&self, asset: Asset) -> Result<Arc<Snapshot>> {
        if let Some((at, snap)) = self
            .snapshots
            .lock()
            .expect("snapshot mutex poisoned")
            .get(&asset)
            && at.elapsed() < SNAPSHOT_TTL
        {
            return Ok(snap.clone());
        }
        let snap = Arc::new(snapshot::build(
            &self.store,
            asset,
            now(),
            self.whale_enabled(),
        )?);
        self.snapshots
            .lock()
            .expect("snapshot mutex poisoned")
            .insert(asset, (Instant::now(), snap.clone()));
        Ok(snap)
    }
}
