use crate::consts::*;
use crate::utils::{main_log, normalize};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// Process-local counter guaranteeing each temp file name is unique even when
/// two saves land within the same wall-clock tick (CG1).
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Config schema for favorites/history entry keys. v1 stored raw stream URLs;
/// v2 stores stable keys (`id:<tvg-id>` / `name:<norm-name>` / `radio:<id>` /
/// `url:<url>`), so rotating a provider link no longer wipes saved lists.
pub const ENTRY_SCHEMA: u32 = 2;

// ─── Entry Keys ──────────────────────────────────────────────────────────────
//
// Why: favorites/history used to be keyed by the stream URL. Any playlist link
// rotation (provider moves host, token is reissued) orphaned every saved entry.
// A key derived from channel identity survives such a change; the URL is kept
// only as a fallback for entries that are no longer in the current playlist.

/// Stable key for a channel: tvg-id first, then normalized name, then URL.
pub fn channel_key(ch: &Channel) -> String {
    if let Some(id) = ch
        .tvg_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return format!("id:{id}");
    }
    let norm = ch.norm_name.trim();
    if !norm.is_empty() {
        return format!("name:{norm}");
    }
    url_entry_key(&ch.url)
}

/// Stable key for a radio station (the provider's numeric id).
pub fn radio_key(st: &RadioStation) -> String {
    if st.id.trim().is_empty() {
        url_entry_key(&st.stream)
    } else {
        format!("radio:{}", st.id.trim())
    }
}

/// Fallback key for anything that has no identity beyond its URL.
pub fn url_entry_key(url: &str) -> String {
    format!("url:{}", url.trim())
}

// ─── Config ──────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    // Every field has a serde default so a hand-edited or older config missing
    // one key does not get rejected wholesale (and renamed to .bak).
    #[serde(default)]
    pub playlist_url: String,
    #[serde(default = "default_epg_url")]
    pub epg_url: String,
    #[serde(default = "default_theme")]
    pub theme_color: (u8, u8, u8),
    /// Stable entry keys of favorite channels/stations (see `channel_key`).
    #[serde(default)]
    pub favorites: HashSet<String>,
    /// Stable entry keys, oldest first, capped at 200.
    #[serde(default)]
    pub history: Vec<String>,
    /// Entry key → display name cache (renders without a loaded playlist).
    #[serde(default)]
    pub channel_names: HashMap<String, String>,
    /// Entry key → last known stream URL, used when the entry is not present in
    /// the current playlist and as the fallback for `url:` keys.
    #[serde(default)]
    pub entry_urls: HashMap<String, String>,
    /// 0/absent in configs written before stable keys; see `ENTRY_SCHEMA`.
    #[serde(default)]
    pub entry_schema: u32,
    #[serde(default = "default_fullscreen")]
    pub video_fullscreen: bool,
    #[serde(default = "default_geometry")]
    pub video_geometry: String,
    #[serde(default)]
    pub local_dir: String,
    /// Selected AI model: a `MODEL_CATALOG` id token (e.g. "deepseek",
    /// "gemini", "agy:claude-opus-4-6"). Resolved via `ai::resolve_choice`,
    /// which migrates legacy values and never panics. Stored as `String` so the
    /// serde_json config schema is unchanged (no cache/bincode bump needed).
    #[serde(default)]
    pub llm_provider: String,
    /// mtime of config.json as of our last load/save. Runtime only; used to
    /// detect an edit made by another process before we overwrite it.
    #[serde(skip)]
    pub last_mtime: Option<SystemTime>,
    /// Where this config was loaded from. Runtime only; migration uses it to
    /// place the pre-migration backup next to the real file.
    #[serde(skip)]
    pub config_path: Option<std::path::PathBuf>,
}

fn default_fullscreen() -> bool {
    true
}
fn default_geometry() -> String {
    "1280x720".into()
}
fn default_epg_url() -> String {
    RECOMMENDED_EPG.into()
}
fn default_theme() -> (u8, u8, u8) {
    (0, 255, 255)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            playlist_url: String::new(),
            epg_url: RECOMMENDED_EPG.into(),
            theme_color: (0, 255, 255),
            favorites: HashSet::new(),
            history: Vec::new(),
            channel_names: HashMap::new(),
            entry_urls: HashMap::new(),
            entry_schema: ENTRY_SCHEMA,
            video_fullscreen: true,
            video_geometry: "1280x720".into(),
            local_dir: String::new(),
            llm_provider: String::new(),
            last_mtime: None,
            config_path: None,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        Self::load_from(&get_config_json_path())
    }

    /// Load config from an explicit path. On a parse error the corrupt file is
    /// renamed to `<name>.bak` (logged loudly) BEFORE falling back to Default, so
    /// the next save() cannot silently overwrite the user's favorites/history.
    pub fn load_from(path: &Path) -> Self {
        let raw = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => return Self::default(), // no file yet — first run
        };
        match serde_json::from_str::<Config>(&raw) {
            Ok(mut cfg) => {
                cfg.last_mtime = mtime_of(path);
                cfg.config_path = Some(path.to_path_buf());
                cfg
            }
            Err(e) => {
                let bak = path.with_extension("json.bak");
                match fs::rename(path, &bak) {
                    Ok(()) => main_log(&format!(
                        "[config] parse error ({e}) — corrupt config backed up to {}",
                        bak.display()
                    )),
                    Err(re) => main_log(&format!(
                        "[config] parse error ({e}) — FAILED to back up corrupt config: {re}"
                    )),
                }
                Self::default()
            }
        }
    }

    /// Durable save (fsync before rename) — use for favorites/settings changes.
    pub fn save(&mut self) -> Result<()> {
        self.save_inner(true)
    }

    /// Non-durable save (skips fsync) — used for high-frequency history-only
    /// updates where a crash losing the very last watched entry is acceptable and
    /// an fsync per channel change is not worth the I/O stall.
    pub fn save_fast(&mut self) -> Result<()> {
        self.save_inner(false)
    }

    fn save_inner(&mut self, durable: bool) -> Result<()> {
        // O(n) prune: only keys still referenced by favorites/history keep
        // their cached name and fallback URL.
        let keep: HashSet<&String> = self.history.iter().chain(self.favorites.iter()).collect();
        self.channel_names.retain(|key, _| keep.contains(key));
        self.entry_urls.retain(|key, _| keep.contains(key));

        use std::io::Write;
        let dir = get_config_dir();
        fs::create_dir_all(&dir)?;
        let path = get_config_json_path();

        // Another process edited config.json since our last load/save: keep a
        // copy of its version before overwriting it, so no edit is lost silently.
        if let Some(last) = self.last_mtime
            && let Some(now) = mtime_of(&path)
            && now > last
        {
            let stamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let conflict = path.with_extension(format!("json.conflict.{stamp}.bak"));
            match fs::copy(&path, &conflict) {
                Ok(_) => main_log(&format!(
                    "[config] on-disk config changed while running — previous version kept as {}",
                    conflict.display()
                )),
                Err(e) => main_log(&format!("[config] conflict backup failed: {e}")),
            }
        }

        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = path.with_extension(format!("tmp.{}.{}", std::process::id(), seq));
        {
            let mut f = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)?;
            f.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
            if durable {
                f.sync_all()?;
            }
        }
        fs::rename(&tmp, &path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        self.last_mtime = mtime_of(&path);
        Ok(())
    }

    // ─── Entry accessors ──────────────────────────────────────────────────

    pub fn entry_name<'a>(&'a self, key: &'a str) -> &'a str {
        self.channel_names
            .get(key)
            .map(String::as_str)
            .unwrap_or(key)
    }

    pub fn entry_url<'a>(&'a self, key: &'a str) -> Option<&'a str> {
        if let Some(url) = self.entry_urls.get(key) {
            return Some(url);
        }
        key.strip_prefix("url:")
    }

    pub fn is_favorite(&self, key: &str) -> bool {
        self.favorites.contains(key)
    }

    pub fn history_push(&mut self, key: &str, url: &str, name: &str) {
        self.history.retain(|k| k != key);
        self.history.push(key.to_string());
        if self.history.len() > 200 {
            self.history.drain(0..self.history.len() - 200);
        }
        self.remember_entry(key, url, name);
        // History is high-frequency and low-value on crash — skip fsync.
        if let Err(e) = self.save_fast() {
            main_log(&format!("[config] save failed: {e}"));
        }
    }

    pub fn favorite_add(&mut self, key: &str, url: &str, name: &str) {
        self.favorites.insert(key.to_string());
        self.remember_entry(key, url, name);
        if let Err(e) = self.save() {
            main_log(&format!("[config] save failed: {e}"));
        }
    }

    pub fn favorite_remove(&mut self, key: &str) {
        self.favorites.remove(key);
        if let Err(e) = self.save() {
            main_log(&format!("[config] save failed: {e}"));
        }
    }

    fn remember_entry(&mut self, key: &str, url: &str, name: &str) {
        self.entry_urls.insert(key.to_string(), url.to_string());
        if !name.is_empty() {
            self.channel_names.insert(key.to_string(), name.to_string());
        }
    }

    // ─── Stable-key migration ─────────────────────────────────────────────

    /// Bring entry keys up to `ENTRY_SCHEMA`: v1 URL-keyed lists are remapped
    /// to stable keys and `url:` keys are upgraded whenever the current data
    /// can resolve them. Returns true when the config changed and must be saved.
    pub fn migrate_entries(&mut self, data: &AppData) -> bool {
        let mut changed = false;
        if self.entry_schema < ENTRY_SCHEMA {
            let has_entries = !self.favorites.is_empty()
                || !self.history.is_empty()
                || !self.channel_names.is_empty();
            if has_entries && let Some(path) = &self.config_path {
                let stamp = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                if path.exists() {
                    let bak = path.with_extension(format!("json.pre-keys.{stamp}.bak"));
                    if let Err(e) = fs::copy(path, &bak) {
                        main_log(&format!("[config] pre-migration backup failed: {e}"));
                    }
                }
            }
            self.upgrade_legacy_entries(data);
            self.entry_schema = ENTRY_SCHEMA;
            changed = true;
        }
        if self.rekey_entries(data) {
            changed = true;
        }
        changed
    }

    /// Remap v1 URL-keyed entries to stable keys using the loaded cache.
    fn upgrade_legacy_entries(&mut self, data: &AppData) {
        let old_names = std::mem::take(&mut self.channel_names);
        let old_favorites = std::mem::take(&mut self.favorites);
        let old_history = std::mem::take(&mut self.history);
        let old_urls = std::mem::take(&mut self.entry_urls);

        let mut lookup: HashMap<&str, (String, String)> = HashMap::new();
        for ch in &data.channels {
            lookup
                .entry(ch.url.as_str())
                .or_insert_with(|| (channel_key(ch), ch.name.clone()));
        }
        for st in &data.radio {
            let key = radio_key(st);
            lookup
                .entry(st.stream.as_str())
                .or_insert_with(|| (key.clone(), st.title.clone()));
            for url in st.quality_urls.values() {
                lookup
                    .entry(url.as_str())
                    .or_insert_with(|| (key.clone(), st.title.clone()));
            }
        }

        let resolve = |url: &str| -> (String, String) {
            if let Some((key, name)) = lookup.get(url) {
                return (key.clone(), name.clone());
            }
            let name = old_names.get(url).cloned().unwrap_or_default();
            (url_entry_key(url), name)
        };

        for url in old_favorites {
            let (key, name) = resolve(&url);
            self.remember_entry(&key, &url, &name);
            self.favorites.insert(key);
        }

        for url in old_history {
            let (key, name) = resolve(&url);
            self.remember_entry(&key, &url, &name);
            // Keep the newest occurrence only, preserving order.
            self.history.retain(|k| k != &key);
            self.history.push(key);
        }

        for (url, name) in &old_names {
            let (key, resolved_name) = resolve(url);
            let name = if resolved_name.is_empty() {
                name.clone()
            } else {
                resolved_name
            };
            self.entry_urls
                .entry(key.clone())
                .or_insert_with(|| url.clone());
            if !name.is_empty() {
                self.channel_names.entry(key).or_insert(name);
            }
        }

        // Half-migrated key→URL pairs from an interrupted run are preserved.
        for (key, url) in old_urls {
            self.entry_urls.entry(key).or_insert(url);
        }
    }

    /// Upgrade `url:` keys to identity keys for entries present in the data.
    pub fn rekey_entries(&mut self, data: &AppData) -> bool {
        let keys: Vec<String> = self
            .favorites
            .iter()
            .chain(self.history.iter())
            .chain(self.channel_names.keys())
            .chain(self.entry_urls.keys())
            .cloned()
            .collect();
        let mut changed = false;
        for key in keys {
            if !key.starts_with("url:") {
                continue;
            }
            let url = match self.entry_urls.get(&key) {
                Some(u) => u.clone(),
                None => key.trim_start_matches("url:").to_string(),
            };
            // Best effort, in order: the exact URL, then an unambiguous name
            // match (survives a provider that reissued every stream URL), and
            // finally the remembered display name against radio titles.
            let entry_name = self.channel_names.get(&key).cloned();
            let new_key = data
                .url_index
                .get(&url)
                .map(|&i| channel_key(&data.channels[i]))
                .or_else(|| {
                    data.radio
                        .iter()
                        .find(|st| st.stream == url || st.quality_urls.values().any(|u| u == &url))
                        .map(radio_key)
                })
                .or_else(|| {
                    entry_name
                        .as_deref()
                        .and_then(|n| resolve_channel_by_name(data, n))
                })
                .or_else(|| {
                    entry_name
                        .as_deref()
                        .and_then(|n| resolve_radio_by_name(data, n))
                });
            let Some(new_key) = new_key else { continue };
            if new_key == key {
                continue;
            }

            if self.favorites.remove(&key) {
                self.favorites.insert(new_key.clone());
            }
            for slot in self.history.iter_mut().filter(|k| *k == &key) {
                *slot = new_key.clone();
            }
            if let Some(name) = self.channel_names.remove(&key) {
                self.channel_names.entry(new_key.clone()).or_insert(name);
            }
            if let Some(url) = self.entry_urls.remove(&key) {
                self.entry_urls.entry(new_key).or_insert(url);
            }
            changed = true;
        }

        if changed {
            // Rekeying can collapse two history entries into one.
            let mut seen = HashSet::new();
            self.history.reverse();
            self.history.retain(|k| seen.insert(k.clone()));
            self.history.reverse();
        }
        changed
    }
}

/// Resolve a display name to a channel key, but only when exactly one channel
/// matches — an ambiguous name must not silently bind the wrong stream.
fn resolve_channel_by_name(data: &AppData, name: &str) -> Option<String> {
    let norm = normalize(name);
    if norm.is_empty() {
        return None;
    }
    let mut matches = data.channels.iter().filter(|ch| ch.norm_name == norm);
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(channel_key(first))
}

/// Same for radio stations, matched on their title.
fn resolve_radio_by_name(data: &AppData, name: &str) -> Option<String> {
    let norm = normalize(name);
    if norm.is_empty() {
        return None;
    }
    let mut matches = data.radio.iter().filter(|st| normalize(&st.title) == norm);
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(radio_key(first))
}

fn mtime_of(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

// ─── Data Model ──────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Channel {
    pub name: String,
    pub group: String,
    pub url: String,
    pub tvg_id: Option<String>,
    pub norm_name: String,
    #[serde(default)]
    pub catchup_days: u32,
    #[serde(default)]
    pub name_lower: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RadioStation {
    pub id: String,
    pub title: String,
    pub stream: String,
    #[serde(default)]
    pub quality_urls: HashMap<String, String>,
    pub genres: Vec<String>,
    pub provider: String,
    pub track: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EpgProgram {
    pub start: i64,
    pub stop: i64,
    pub title: String,
    pub desc: String,
}

#[derive(Serialize, Deserialize, Default)]
pub struct AppData {
    pub channels: Vec<Channel>,
    pub radio: Vec<RadioStation>,
    pub radio_groups: Vec<String>,
    pub groups: Vec<String>,
    pub epg: HashMap<String, Vec<EpgProgram>>,
    pub name_to_id: HashMap<String, String>,
    #[serde(default)]
    pub group_counts: HashMap<String, usize>,
    /// url → channel index, for O(1) reverse lookups. Derived state, never
    /// persisted (rebuilt on load via `build_indices`), so the on-disk cache
    /// format is unchanged.
    #[serde(skip)]
    pub url_index: HashMap<String, usize>,
    /// stable channel key → channel index, for favorites/history resolution.
    #[serde(skip)]
    pub key_index: HashMap<String, usize>,
    /// stable radio key → radio index.
    #[serde(skip)]
    pub radio_key_index: HashMap<String, usize>,
    /// radio genre (uppercased) → station count, for the genre list. Derived,
    /// not persisted.
    #[serde(skip)]
    pub radio_group_counts: HashMap<String, usize>,
}

impl AppData {
    /// Rebuild the derived reverse-lookup indices after (de)serialization.
    pub fn build_indices(&mut self) {
        self.url_index = self
            .channels
            .iter()
            .enumerate()
            .map(|(i, ch)| (ch.url.clone(), i))
            .collect();
        self.key_index.clear();
        for (i, ch) in self.channels.iter().enumerate() {
            self.key_index.entry(channel_key(ch)).or_insert(i);
        }
        self.radio_key_index = self
            .radio
            .iter()
            .enumerate()
            .map(|(i, st)| (radio_key(st), i))
            .collect();
        let mut radio_counts: HashMap<String, usize> = HashMap::new();
        for st in &self.radio {
            for g in &st.genres {
                *radio_counts.entry(g.clone()).or_insert(0) += 1;
            }
        }
        self.radio_group_counts = radio_counts;
    }
}

#[derive(Serialize, Deserialize)]
pub struct CacheContainer {
    pub version: u32,
    pub data: AppData,
}

// ─── Cache Metadata ──────────────────────────────────────────────────────────

/// Out-of-band description of the current cache: which sources produced it and
/// when. Kept in a separate JSON file so the bincode cache layout stays intact
/// (no schema bump / forced re-download on upgrade).
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
pub struct CacheMeta {
    pub playlist_url: String,
    pub epg_url: String,
    pub fetched_at: i64,
    pub channels: usize,
    pub epg_channels: usize,
    #[serde(default)]
    pub epg_warning: Option<String>,
}

impl CacheMeta {
    pub fn load() -> Option<Self> {
        let raw = fs::read_to_string(get_cache_meta_path()).ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub fn save(&self) -> Result<()> {
        let path = get_cache_meta_path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[derive(PartialEq, Clone, Copy)]
pub enum Screen {
    MainMenu,
    CatList,
    ChanList,
    Detail,
    RadioCatList,
    RadioList,
    Settings,
    SettingsEdit(usize),
    Updating,
    LocalList,
    Favorites,
    History,
    AiChat,
    Confirm,
}

pub const THEME_PRESETS: &[(u8, u8, u8, &str)] = &[
    (0, 255, 255, "Cyan"),
    (255, 0, 255, "Magenta"),
    (0, 255, 128, "Neon Green"),
    (255, 128, 0, "Orange"),
    (128, 0, 255, "Purple"),
    (255, 255, 0, "Yellow"),
    (255, 0, 0, "Red"),
];
