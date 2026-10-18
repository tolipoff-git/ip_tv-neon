// Declarative Settings registry.
//
// Why: the screen order used to be duplicated in four places — the label array,
// the value getter, the apply/toggle match, and the key handler — so adding one
// setting meant touching all four and keeping magic indices in sync. The table
// below is now the single source of truth for order, labels, grouping, and
// which setting is editable vs toggleable vs destructive.
//
// It also owns input validation, so a URL is checked before it is written to
// the config instead of being discovered as broken on the next Update.

use crate::models::Config;
use std::path::Path;

/// Stable identity of a setting, independent of its position in the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingId {
    PlaylistUrl,
    EpgUrl,
    Fullscreen,
    Geometry,
    Theme,
    AiProvider,
    LocalDir,
    ClearHistory,
    ClearFavorites,
}

/// How the screen behaves when the row is activated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKind {
    /// Free text; `Enter` opens the editor.
    Text,
    /// Boolean flipped with `Enter`/`←`/`→`.
    Toggle,
    /// Cycles through a list; `←`/`→` pick the direction.
    Cycle,
    /// Destructive action that always asks for confirmation.
    Action,
}

pub struct Setting {
    pub section: &'static str,
    pub id: SettingId,
    pub label: &'static str,
    pub kind: SettingKind,
    /// Value restored by `r` (only for text settings).
    pub default: Option<&'static str>,
}

pub const SECTION_SOURCES: &str = "SOURCES";
pub const SECTION_PLAYER: &str = "PLAYER";
pub const SECTION_APPEARANCE: &str = "APPEARANCE";
pub const SECTION_AI: &str = "AI";
pub const SECTION_DATA: &str = "DATA";

pub static SETTINGS: &[Setting] = &[
    Setting {
        section: SECTION_SOURCES,
        id: SettingId::PlaylistUrl,
        label: "Playlist URL / file",
        kind: SettingKind::Text,
        default: None,
    },
    Setting {
        section: SECTION_SOURCES,
        id: SettingId::EpgUrl,
        label: "EPG URL",
        kind: SettingKind::Text,
        default: Some(crate::consts::RECOMMENDED_EPG),
    },
    Setting {
        section: SECTION_SOURCES,
        id: SettingId::LocalDir,
        label: "Local playlists dir",
        kind: SettingKind::Text,
        default: Some(""),
    },
    Setting {
        section: SECTION_PLAYER,
        id: SettingId::Fullscreen,
        label: "Fullscreen",
        kind: SettingKind::Toggle,
        default: None,
    },
    Setting {
        section: SECTION_PLAYER,
        id: SettingId::Geometry,
        label: "Window geometry",
        kind: SettingKind::Text,
        default: Some("1280x720"),
    },
    Setting {
        section: SECTION_APPEARANCE,
        id: SettingId::Theme,
        label: "Theme",
        kind: SettingKind::Cycle,
        default: None,
    },
    Setting {
        section: SECTION_AI,
        id: SettingId::AiProvider,
        label: "AI provider",
        kind: SettingKind::Cycle,
        default: None,
    },
    Setting {
        section: SECTION_DATA,
        id: SettingId::ClearHistory,
        label: "Clear history",
        kind: SettingKind::Action,
        default: None,
    },
    Setting {
        section: SECTION_DATA,
        id: SettingId::ClearFavorites,
        label: "Clear favorites",
        kind: SettingKind::Action,
        default: None,
    },
];

pub fn setting_at(index: usize) -> Option<&'static Setting> {
    SETTINGS.get(index)
}

/// True for a well-formed `http://` or `https://` URL with a non-empty rest.
pub fn is_http_source(value: &str) -> bool {
    let s = value.trim();
    let lower = s.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("https://") {
        r
    } else if let Some(r) = lower.strip_prefix("http://") {
        r
    } else {
        return false;
    };
    !rest.is_empty() && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Expand a leading `~/` using the user's home directory. `Path` itself does
/// not expand it, so a pasted `~/playlists` would silently never match.
pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().into_owned();
    }
    path.to_string()
}

/// Validate and normalize the playlist source: an http(s) URL or an existing
/// local `.m3u`/`.m3u8` file. Returns the value to store.
pub fn validate_playlist_input(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("Playlist is empty — enter an http(s) URL or a local .m3u file".into());
    }
    if is_http_source(s) {
        return Ok(s.to_string());
    }
    if s.contains("://") {
        return Err("Only http:// or https:// URLs are supported".into());
    }
    let path = expand_tilde(s);
    if !Path::new(&path).is_file() {
        return Err(format!("Playlist file not found: {path}"));
    }
    Ok(path)
}

/// Validate and normalize the EPG URL. XMLTV is fetched over HTTP only.
pub fn validate_epg_input(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("EPG URL is empty — enter an http(s) XMLTV URL".into());
    }
    if !is_http_source(s) {
        return Err("EPG must be an http:// or https:// URL".into());
    }
    Ok(s.to_string())
}

/// Validate an mpv `--geometry` value of the form `WIDTHxHEIGHT`.
pub fn validate_geometry(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| "Use WIDTHxHEIGHT, e.g. 1280x720".to_string())?;
    let parse = |v: &str| v.parse::<u32>().ok();
    match (parse(w), parse(h)) {
        (Some(w), Some(h)) if (320..=16384).contains(&w) && (200..=16384).contains(&h) => {
            Ok(format!("{w}x{h}"))
        }
        _ => Err("Geometry must be WIDTHxHEIGHT within 320..16384, e.g. 1280x720".into()),
    }
}

/// Validate the local playlists directory. Empty means "use the defaults"
/// (~/, ~/Downloads, ~/Videos); otherwise it must be an existing directory.
pub fn validate_local_dir(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(String::new());
    }
    let path = expand_tilde(s);
    if !Path::new(&path).is_dir() {
        return Err(format!("Directory not found: {path}"));
    }
    Ok(path)
}

/// Validate a text setting by id and return the normalized value.
pub fn validate_text(id: SettingId, raw: &str) -> Result<String, String> {
    match id {
        SettingId::PlaylistUrl => validate_playlist_input(raw),
        SettingId::EpgUrl => validate_epg_input(raw),
        SettingId::Geometry => validate_geometry(raw),
        SettingId::LocalDir => validate_local_dir(raw),
        _ => Ok(raw.trim().to_string()),
    }
}

/// True when saving this setting means the cached playlist/EPG is stale.
pub fn invalidates_sources(id: SettingId) -> bool {
    matches!(id, SettingId::PlaylistUrl | SettingId::EpgUrl)
}

/// Default value for `r` (reset) on text rows: the registry entry if it has
/// one, otherwise the current value (a no-op reset for the playlist).
pub fn reset_value(cfg: &Config, id: SettingId) -> Option<String> {
    let setting = SETTINGS.iter().find(|s| s.id == id)?;
    match setting.default {
        Some(default) => Some(default.to_string()),
        None => match id {
            SettingId::PlaylistUrl => Some(cfg.playlist_url.clone()),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_ids_are_unique() {
        for (i, a) in SETTINGS.iter().enumerate() {
            for b in &SETTINGS[i + 1..] {
                assert_ne!(a.id, b.id, "duplicate setting id {:?}", a.id);
            }
            assert!(!a.label.is_empty());
        }
        assert_eq!(SETTINGS.iter().map(|s| s.id).collect::<Vec<_>>().len(), 9);
    }

    #[test]
    fn sections_are_contiguous_in_screen_order() {
        // The renderer prints a header whenever the section changes, so a
        // section must never reappear after a different one.
        let mut seen: Vec<&str> = Vec::new();
        for setting in SETTINGS {
            if seen.last() != Some(&setting.section) {
                assert!(
                    !seen.contains(&setting.section),
                    "section {} appears twice — regroup SETTINGS",
                    setting.section
                );
                seen.push(setting.section);
            }
        }
    }

    #[test]
    fn http_source_requires_scheme_and_host() {
        assert!(is_http_source("https://example.com/a.m3u"));
        assert!(is_http_source("  HTTP://127.0.0.1:8095/neon.m3u "));
        assert!(!is_http_source("http://"));
        assert!(!is_http_source("ftp://example.com/a"));
        assert!(!is_http_source("https://exa mple.com"));
        assert!(!is_http_source("/home/admin/a.m3u"));
    }

    #[test]
    fn playlist_accepts_http_and_rejects_garbage() {
        assert_eq!(
            validate_playlist_input(" http://h/l.m3u ").unwrap(),
            "http://h/l.m3u"
        );
        assert!(validate_playlist_input("").unwrap_err().contains("empty"));
        assert!(
            validate_playlist_input("ftp://h/l.m3u")
                .unwrap_err()
                .contains("http")
        );
        assert!(
            validate_playlist_input("/nope/missing.m3u")
                .unwrap_err()
                .contains("not found")
        );
    }

    #[test]
    fn epg_requires_http() {
        assert!(validate_epg_input("https://epg.one/epg.xml.gz").is_ok());
        assert!(validate_epg_input("/tmp/epg.xml").is_err());
        assert!(validate_epg_input("").is_err());
    }

    #[test]
    fn geometry_is_normalized_and_bounded() {
        assert_eq!(validate_geometry(" 1280X720 ").unwrap(), "1280x720");
        assert!(validate_geometry("1280").is_err());
        assert!(validate_geometry("10x10").is_err());
        assert!(validate_geometry("abcxdef").is_err());
    }

    #[test]
    fn local_dir_must_exist_unless_empty() {
        assert_eq!(validate_local_dir("").unwrap(), "");
        assert!(validate_local_dir("/definitely/not/here").is_err());
        assert!(validate_local_dir("/tmp").is_ok());
    }
}
