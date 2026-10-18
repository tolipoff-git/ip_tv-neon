// ─── Imports ─────────────────────────────────────────────────────────────────

use crate::ai::{AiSearchResult, ChatMsg};
use crate::epg::{find_epg_id, load_data};
use crate::models::{
    AppData, CacheMeta, Config, EpgProgram, Screen, THEME_PRESETS, channel_key, radio_key,
    url_entry_key,
};
use crate::settings::{self, SettingId, SettingKind};
use crate::text_input::TextInput;
use crate::utils::main_log;
use ratatui::widgets::ListState;
use std::path::PathBuf;

// ─── Detail Screen Sub-State ────────────────────────────────────────────────

#[derive(Default)]
pub struct DetailState {
    pub channel: Option<usize>,
    pub programs: Vec<EpgProgram>,
    pub current_idx: Option<usize>,
    pub return_screen: Option<Screen>,
}

// ─── AI Chat Sub-State ───────────────────────────────────────────────────────

#[derive(Default)]
pub struct AiState {
    pub query: String,
    pub results: Vec<AiSearchResult>,
    pub loading: bool,
    pub chat_history: Vec<ChatMsg>,
    pub focus_results: bool,
}

// ─── Navigation Sub-State ────────────────────────────────────────────────────

#[derive(Default)]
pub struct NavState {
    pub m_state: ListState,
    pub cat_state: ListState,
    pub ch_state: ListState,
    pub r_state: ListState,
    pub r_cat_state: ListState,
    pub d_state: ListState,
    pub fav_state: ListState,
    pub hist_state: ListState,
    pub set_state: ListState,
    /// First visible line index of the settings list (manual scroll).
    pub set_offset: usize,
    /// First visible line index of the help screen (manual scroll).
    pub help_offset: usize,
    pub epg_state: ListState,
    pub ai_state: ListState,
    pub filtered: Vec<usize>,
    /// Scroll offset (first visible row) for the channel list, managed manually so
    /// only the visible window of ListItems is built each frame.
    pub ch_offset: usize,
    pub filtered_radio: Vec<usize>,
    pub selected_group: String,
    pub selected_radio_genre: String,
    pub search: String,
}

// ─── Confirmation Dialog ─────────────────────────────────────────────────────

/// What a `y` on the confirmation dialog performs. Destructive or expensive
/// actions never fire on a single keypress anymore.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmAction {
    ClearHistory,
    ClearFavorites,
    UpdateData,
}

pub struct Confirm {
    pub prompt: String,
    pub action: ConfirmAction,
    /// Screen to return to when the dialog closes.
    pub return_to: Screen,
}

// ─── App State ───────────────────────────────────────────────────────────────

pub struct App {
    pub config: Config,
    pub data: AppData,
    pub screen: Screen,
    pub nav: NavState,
    pub quit: bool,
    pub suspended: bool,
    pub local_files: Vec<PathBuf>,
    pub player: crate::player::PlayerController,
    pub needs_redraw: bool,
    pub debug: bool,
    pub status_msg: Option<String>,
    pub detail: DetailState,
    pub ai: AiState,
    /// Whether the agy CLI binary was found at startup (drives the Settings hint).
    pub agy_available: bool,
    /// Text buffer of the setting currently being edited.
    pub input: TextInput,
    /// Validation error of the last edit attempt (shown in the settings footer).
    pub setting_error: Option<String>,
    /// Active confirmation dialog, if any.
    pub confirm: Option<Confirm>,
    /// Metadata of the cache currently on disk (source URLs + fetch time).
    pub cache_meta: Option<CacheMeta>,
    /// True when a source URL changed after the last successful update.
    pub needs_source_update: bool,
    /// Screen to return to when the help overlay closes.
    pub help_return: Screen,
}

/// Accept only http/https media URLs (CG4). Case-insensitive on the scheme.
pub fn is_http_url(url: &str) -> bool {
    let u = url.trim_start();
    let lower = u.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Build a flussonic catch-up (archive) URL.
///
/// `utc` is the requested archive start (program start time). `lutc` must be
/// the *current* live-edge timestamp (i.e. "now"), NOT the program's stop time:
/// flussonic interprets `lutc` as the current wall-clock of the live stream and
/// uses it to anchor the catch-up window. Passing the program stop made the
/// provider serve the live edge / an empty window instead of the past archive.
fn build_archive_url(base: &str, utc: i64, lutc: i64) -> String {
    // Tokenized stream URLs already carry a query string (`?token=…`); an
    // unconditional `?` would corrupt them. Pick the correct separator.
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}utc={utc}&lutc={lutc}")
}

// ─── Constructor & Data Loading ──────────────────────────────────────────────

impl App {
    pub fn new(mut config: Config) -> Self {
        let data = load_data();

        // Bring favorites/history from URL keys to stable keys (v1 → v2) and
        // upgrade any `url:` entries the cache can resolve by identity.
        if config.migrate_entries(&data)
            && let Err(e) = config.save()
        {
            main_log(&format!("[config] save after key migration failed: {e}"));
        }

        let mut app = Self {
            config,
            data,
            screen: Screen::MainMenu,
            nav: NavState::default(),
            quit: false,
            suspended: false,
            local_files: Vec::new(),
            player: crate::player::PlayerController::new(),
            needs_redraw: true,
            debug: false,
            status_msg: None,
            detail: DetailState::default(),
            ai: AiState::default(),
            agy_available: crate::ai::agy_available(),
            input: TextInput::default(),
            setting_error: None,
            confirm: None,
            cache_meta: CacheMeta::load(),
            needs_source_update: false,
            help_return: Screen::MainMenu,
        };
        app.nav.m_state.select(Some(0));
        app.nav.cat_state.select(Some(0));
        app.nav.r_cat_state.select(Some(0));
        app.nav.set_state.select(Some(0));
        app.refresh_source_state();
        app.backfill_channel_names();
        app
    }

    pub fn reload_data(&mut self) {
        self.data = load_data();
        if self.config.migrate_entries(&self.data)
            && let Err(e) = self.config.save()
        {
            main_log(&format!("[config] save after key migration failed: {e}"));
        }
        self.cache_meta = CacheMeta::load();
        self.refresh_source_state();
        self.backfill_channel_names();
    }

    /// Recompute whether the cache was built from the currently configured
    /// sources. Drives the "update pending" hints in the UI.
    pub fn refresh_source_state(&mut self) {
        self.needs_source_update = self.cache_meta.as_ref().is_some_and(|meta| {
            meta.playlist_url != self.config.playlist_url.trim()
                || meta.epg_url != self.config.epg_url.trim()
        });
    }

    /// Fill `channel_names` for favorites/history entries so lists render even
    /// when the current playlist lacks the channel.
    fn backfill_channel_names(&mut self) {
        let mut dirty = false;
        let keys: Vec<String> = self
            .config
            .favorites
            .iter()
            .chain(self.config.history.iter())
            .cloned()
            .collect();
        for key in &keys {
            if self.config.channel_names.contains_key(key) {
                continue;
            }
            if let Some((_, name)) = self.entry_location(key).map(|(u, n, _)| (u, n))
                && !name.is_empty()
            {
                self.config.channel_names.insert(key.clone(), name);
                dirty = true;
            }
        }
        if dirty && let Err(e) = self.config.save() {
            main_log(&format!("[config] save failed: {e}"));
        }
    }

    // ─── Help overlay ────────────────────────────────────────────────────

    /// Toggle the key-reference overlay, remembering where to return.
    pub fn toggle_help(&mut self) {
        if self.screen == Screen::Help {
            self.close_help();
        } else {
            self.help_return = self.screen;
            self.nav.help_offset = 0;
            self.screen = Screen::Help;
        }
    }

    pub fn close_help(&mut self) {
        self.screen = match self.help_return {
            Screen::Help => Screen::MainMenu,
            other => other,
        };
    }

    // ─── Stable entry keys (favorites / history) ─────────────────────────

    pub fn channel_key_at(&self, idx: usize) -> Option<String> {
        self.data.channels.get(idx).map(channel_key)
    }

    pub fn is_channel_favorite(&self, idx: usize) -> bool {
        self.channel_key_at(idx)
            .is_some_and(|key| self.config.is_favorite(&key))
    }

    pub fn toggle_favorite_channel(&mut self, idx: usize) {
        let Some(ch) = self.data.channels.get(idx) else {
            return;
        };
        let key = channel_key(ch);
        let (url, name) = (ch.url.clone(), ch.name.clone());
        if self.config.is_favorite(&key) {
            self.config.favorite_remove(&key);
            self.status_msg = Some(format!("Removed from favorites: {name}"));
        } else {
            self.config.favorite_add(&key, &url, &name);
            self.status_msg = Some(format!("Added to favorites: {name}"));
        }
    }

    pub fn toggle_favorite_radio(&mut self, idx: usize) {
        let Some(st) = self.data.radio.get(idx) else {
            return;
        };
        let key = radio_key(st);
        let (url, name) = (st.stream.clone(), st.title.clone());
        if self.config.is_favorite(&key) {
            self.config.favorite_remove(&key);
            self.status_msg = Some(format!("Removed from favorites: {name}"));
        } else {
            self.config.favorite_add(&key, &url, &name);
            self.status_msg = Some(format!("Added to favorites: {name}"));
        }
    }

    /// Resolve an entry key to the URL/name to play. Prefers the current
    /// playlist (so a rotated provider link resolves to the new URL), falls
    /// back to the URL remembered when the entry was saved.
    pub fn entry_location(&self, key: &str) -> Option<(String, String, bool)> {
        if let Some(&idx) = self.data.key_index.get(key)
            && let Some(ch) = self.data.channels.get(idx)
        {
            return Some((ch.url.clone(), ch.name.clone(), false));
        }
        if let Some(&idx) = self.data.radio_key_index.get(key)
            && let Some(st) = self.data.radio.get(idx)
        {
            return Some((st.stream.clone(), st.title.clone(), true));
        }
        let url = self.config.entry_url(key)?.to_string();
        let name = self.config.entry_name(key).to_string();
        let is_radio = self
            .data
            .radio
            .iter()
            .any(|st| st.stream == url || st.quality_urls.values().any(|u| u == &url));
        Some((url, name, is_radio))
    }

    /// Play a favorites/history entry by its stable key.
    pub fn play_entry(&mut self, key: &str) {
        let Some((url, name, is_radio)) = self.entry_location(key) else {
            self.status_msg = Some(format!("Entry unavailable: {key}"));
            return;
        };
        if is_radio {
            self.run_radio(&url, &name, "");
        } else {
            self.run_video(&url, &name, "");
        }
        self.config.history_push(key, &url, &name);
    }

    pub fn sorted_favorites(&self) -> Vec<String> {
        let mut favs: Vec<String> = self.config.favorites.iter().cloned().collect();
        favs.sort_by(|a, b| self.config.entry_name(a).cmp(self.config.entry_name(b)));
        favs
    }

    /// Record playback under the channel's stable key (not its URL).
    fn remember_playback(&mut self, url: &str, name: &str) {
        let key = self
            .data
            .url_index
            .get(url)
            .map(|&i| channel_key(&self.data.channels[i]))
            .or_else(|| {
                self.data
                    .radio
                    .iter()
                    .find(|st| st.stream == url || st.quality_urls.values().any(|u| u == url))
                    .map(radio_key)
            })
            .unwrap_or_else(|| url_entry_key(url));
        self.config.history_push(&key, url, name);
    }

    pub fn stop_all(&mut self) {
        self.player.stop_all();
        self.suspended = false;
    }

    // ─── Radio VU + Marquee Tick (called every 50ms) ──────────────────────

    /// Advance VU-meter simulation and marquee scroll by one 50ms tick.
    /// Should only be called while radio_ipc.is_some().
    pub fn tick_radio(&mut self) {
        self.player.tick_radio();
    }

    // ─── Detail / EPG Playback ────────────────────────────────────────────

    pub fn open_detail(&mut self, channel_idx: usize) {
        if channel_idx >= self.data.channels.len() {
            return;
        }
        self.detail.return_screen = Some(self.screen);
        let ch = &self.data.channels[channel_idx];
        self.detail.channel = Some(channel_idx);

        let now = chrono::Utc::now().timestamp();
        let epg_id = find_epg_id(ch, &self.data);
        let programs: Vec<EpgProgram> = if let Some(id) = epg_id {
            if let Some(progs) = self.data.epg.get(&id) {
                let cutoff = if ch.catchup_days > 0 {
                    now - (ch.catchup_days as i64 * 86400)
                } else {
                    now - 3600
                };
                progs.iter().filter(|p| p.stop > cutoff).cloned().collect()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let current_idx = programs.iter().position(|p| now >= p.start && now < p.stop);
        self.detail.programs = programs;
        self.detail.current_idx = current_idx;

        let select = current_idx.unwrap_or(0);
        self.nav
            .epg_state
            .select(if self.detail.programs.is_empty() {
                None
            } else {
                Some(select)
            });
        self.screen = Screen::Detail;
    }

    pub fn detail_play_selected(&mut self) {
        let ch_idx = match self.detail.channel {
            Some(i) if i < self.data.channels.len() => i,
            _ => return,
        };
        let ch = &self.data.channels[ch_idx];
        let url = ch.url.clone();
        let name = ch.name.clone();

        let epg_idx = self.nav.epg_state.selected();
        let now = chrono::Utc::now().timestamp();

        if let Some(idx) = epg_idx
            && idx < self.detail.programs.len()
        {
            let prog_title = self.detail.programs[idx].title.clone();
            let prog_start = self.detail.programs[idx].start;
            let prog_stop = self.detail.programs[idx].stop;
            let is_current = now >= prog_start && now < prog_stop;
            let is_future = prog_start > now;

            if is_current || is_future {
                self.run_video(&url, &name, &prog_title);
            } else if ch.catchup_days > 0 {
                // lutc = now (live edge), not prog_stop — flussonic catch-up convention.
                let archive_url = build_archive_url(&url, prog_start, now);
                self.run_video(&archive_url, &name, &prog_title);
            } else {
                self.run_video(&url, &name, &prog_title);
            }
            self.remember_playback(&url, &name);
            return;
        }

        self.run_video(&url, &name, "");
        self.remember_playback(&url, &name);
    }

    pub fn detail_play_live(&mut self) {
        let ch_idx = match self.detail.channel {
            Some(i) if i < self.data.channels.len() => i,
            _ => return,
        };
        let ch = &self.data.channels[ch_idx];
        let url = ch.url.clone();
        let name = ch.name.clone();
        self.run_video(&url, &name, "");
        self.remember_playback(&url, &name);
    }

    // ─── MPV Player ──────────────────────────────────────────────────────

    pub fn run_video(&mut self, url: &str, title: &str, sub_title: &str) {
        self.player.run_video(
            url,
            title,
            sub_title,
            &self.config,
            self.debug,
            &mut self.status_msg,
        );
    }

    pub fn run_radio(&mut self, url: &str, title: &str, sub_title: &str) {
        self.player.run_radio(
            url,
            title,
            sub_title,
            &self.config,
            self.debug,
            &mut self.status_msg,
        );
    }

    // ─── AI Playback ────────────────────────────────────────────────────

    pub fn ai_play_selected(&mut self) {
        let idx = match self.nav.ai_state.selected() {
            Some(i) if i < self.ai.results.len() => i,
            _ => return,
        };
        let result = self.ai.results[idx].clone();
        if result.channel_idx >= self.data.channels.len() {
            return;
        }
        let ch = &self.data.channels[result.channel_idx];
        let url = ch.url.clone();
        let name = ch.name.clone();
        let now = chrono::Utc::now().timestamp();

        if result.program.start > 0 {
            let is_current = now >= result.program.start && now < result.program.stop;
            let is_future = result.program.start > now;
            if is_current || is_future {
                self.run_video(&url, &name, &result.program.title);
            } else if ch.catchup_days > 0 {
                // lutc = now (live edge), not program.stop — flussonic catch-up convention.
                let archive_url = build_archive_url(&url, result.program.start, now);
                self.run_video(&archive_url, &name, &result.program.title);
            } else {
                self.run_video(&url, &name, &result.program.title);
            }
        } else {
            self.run_video(&url, &name, "");
        }
        self.remember_playback(&url, &name);
    }

    // ─── Favorites & Filtering ────────────────────────────────────────────

    pub fn update_filter(&mut self) {
        let q = self.nav.search.to_lowercase();
        let group = &self.nav.selected_group;
        self.nav.filtered = self
            .data
            .channels
            .iter()
            .enumerate()
            .filter(|(_, ch)| ch.group == *group)
            .filter(|(_, ch)| q.is_empty() || ch.name_lower.contains(&q))
            .map(|(i, _)| i)
            .collect();
        if self.nav.filtered.is_empty() && !q.is_empty() {
            self.nav.filtered = self
                .data
                .channels
                .iter()
                .enumerate()
                .filter(|(_, ch)| ch.name_lower.contains(&q))
                .map(|(i, _)| i)
                .collect();
        }
        self.nav.ch_state.select(if self.nav.filtered.is_empty() {
            None
        } else {
            Some(0)
        });
        self.needs_redraw = true;
    }

    pub fn update_radio_filter(&mut self) {
        let genre = &self.nav.selected_radio_genre;
        self.nav.filtered_radio = self
            .data
            .radio
            .iter()
            .enumerate()
            .filter(|(_, r)| genre == "All" || r.genres.contains(genre))
            .map(|(i, _)| i)
            .collect();
        self.nav
            .r_state
            .select(if self.nav.filtered_radio.is_empty() {
                None
            } else {
                Some(0)
            });
        self.needs_redraw = true;
    }

    // ─── Settings ────────────────────────────────────────────────────────

    /// Human-readable value shown in the settings list.
    pub fn settings_value(&self, id: SettingId) -> String {
        match id {
            SettingId::PlaylistUrl => {
                if self.config.playlist_url.trim().is_empty() {
                    "— not set —".into()
                } else {
                    self.config.playlist_url.clone()
                }
            }
            SettingId::EpgUrl => {
                if self.config.epg_url.trim().is_empty() {
                    "— not set —".into()
                } else {
                    self.config.epg_url.clone()
                }
            }
            SettingId::Fullscreen => {
                if self.config.video_fullscreen {
                    "ON".into()
                } else {
                    "OFF".into()
                }
            }
            SettingId::Geometry => self.config.video_geometry.clone(),
            SettingId::Theme => {
                let c = self.config.theme_color;
                THEME_PRESETS
                    .iter()
                    .find(|p| (p.0, p.1, p.2) == c)
                    .map(|p| p.3.to_string())
                    .unwrap_or_else(|| format!("({},{},{})", c.0, c.1, c.2))
            }
            SettingId::AiProvider => {
                let mut label = crate::ai::choice_label(&self.config.llm_provider).to_string();
                let backend = crate::ai::resolve_choice(&self.config.llm_provider).backend;
                if backend == crate::ai::Backend::Agy && !self.agy_available {
                    label.push_str(" (agy not found)");
                }
                label
            }
            SettingId::LocalDir => {
                if self.config.local_dir.is_empty() {
                    "defaults (~/, ~/Downloads, ~/Videos)".into()
                } else {
                    self.config.local_dir.clone()
                }
            }
            SettingId::ClearHistory => format!("{} entries", self.config.history.len()),
            SettingId::ClearFavorites => format!("{} entries", self.config.favorites.len()),
        }
    }

    /// Initial buffer for the editor: the stored value without display sugar.
    fn settings_edit_value(&self, id: SettingId) -> String {
        match id {
            SettingId::PlaylistUrl => self.config.playlist_url.clone(),
            SettingId::EpgUrl => self.config.epg_url.clone(),
            SettingId::Geometry => self.config.video_geometry.clone(),
            SettingId::LocalDir => self.config.local_dir.clone(),
            _ => self.settings_value(id),
        }
    }

    /// Open the editor for a text setting. Returns false for non-text rows.
    pub fn settings_begin_edit(&mut self, id: SettingId, kind: SettingKind) -> bool {
        if kind != SettingKind::Text {
            return false;
        }
        self.input = TextInput::new(&self.settings_edit_value(id));
        self.setting_error = None;
        true
    }

    /// Validate and persist the editor buffer. Returns `Ok(true)` when the
    /// change invalidates the cached sources (playlist/EPG URL).
    pub fn settings_commit(&mut self, id: SettingId) -> Result<bool, String> {
        let value = settings::validate_text(id, &self.input.value())?;
        let changed = match id {
            SettingId::PlaylistUrl => value != self.config.playlist_url,
            SettingId::EpgUrl => value != self.config.epg_url,
            SettingId::Geometry => value != self.config.video_geometry,
            SettingId::LocalDir => value != self.config.local_dir,
            _ => false,
        };
        if !changed {
            return Ok(false);
        }
        match id {
            SettingId::PlaylistUrl => self.config.playlist_url = value,
            SettingId::EpgUrl => self.config.epg_url = value,
            SettingId::Geometry => self.config.video_geometry = value,
            SettingId::LocalDir => self.config.local_dir = value,
            _ => {}
        }
        self.config
            .save()
            .map_err(|e| format!("Save failed: {e}"))?;
        if settings::invalidates_sources(id) {
            self.needs_source_update = true;
            return Ok(true);
        }
        Ok(false)
    }

    /// Adjust a toggle/cycle row. `delta` is -1/0/+1 (0 means "Enter").
    pub fn settings_adjust(&mut self, id: SettingId, delta: i32) {
        match id {
            SettingId::Fullscreen => {
                self.config.video_fullscreen = !self.config.video_fullscreen;
            }
            SettingId::Theme => {
                let cur = self.config.theme_color;
                let pos = THEME_PRESETS
                    .iter()
                    .position(|p| (p.0, p.1, p.2) == cur)
                    .unwrap_or(0) as i32;
                let len = THEME_PRESETS.len() as i32;
                let next = ((pos + if delta == 0 { 1 } else { delta }).rem_euclid(len)) as usize;
                let p = THEME_PRESETS[next];
                self.config.theme_color = (p.0, p.1, p.2);
            }
            SettingId::AiProvider => {
                let next = if delta == 0 {
                    crate::ai::next_choice_id(&self.config.llm_provider)
                } else {
                    crate::ai::step_choice_id(&self.config.llm_provider, delta)
                };
                self.config.llm_provider = next.to_string();
            }
            _ => return,
        }
        if let Err(e) = self.config.save() {
            self.setting_error = Some(format!("Save failed: {e}"));
        } else {
            self.setting_error = None;
        }
    }

    /// Restore a text setting to its default (`r`).
    pub fn settings_reset(&mut self, id: SettingId) -> Result<bool, String> {
        let Some(default) = settings::reset_value(&self.config, id) else {
            return Ok(false);
        };
        self.input = TextInput::new(&default);
        self.settings_commit(id)
    }
}

#[cfg(test)]
mod tests {
    use super::build_archive_url;

    #[test]
    fn archive_url_uses_now_as_lutc_not_prog_stop() {
        let base = "http://host/iptv/TOKEN/204/index.m3u8";
        let prog_start = 1_700_000_000_i64;
        let prog_stop = prog_start + 1800; // a past program end
        let now = 1_700_100_000_i64; // current live edge, later than prog_stop

        let url = build_archive_url(base, prog_start, now);

        // utc must stay the program start (the requested archive window).
        assert!(url.contains(&format!("utc={prog_start}")));
        // lutc must be "now", never the program stop — that was the bug.
        assert!(url.ends_with(&format!("lutc={now}")));
        assert!(!url.contains(&format!("lutc={prog_stop}")));
    }

    #[test]
    fn archive_url_appends_with_question_mark_when_no_query() {
        let url = build_archive_url("http://host/204/index.m3u8", 100, 200);
        assert_eq!(url, "http://host/204/index.m3u8?utc=100&lutc=200");
    }

    #[test]
    fn archive_url_uses_ampersand_when_query_present() {
        // Tokenized URLs already carry `?token=…`; the extra params must be joined
        // with `&`, not a second `?` that would corrupt the query string.
        let url = build_archive_url("http://host/204/index.m3u8?token=abc", 100, 200);
        assert_eq!(url, "http://host/204/index.m3u8?token=abc&utc=100&lutc=200");
    }
}
