// Stable-key behavior for favorites/history: the migration from URL keys to
// identity keys, and the resolution of an entry against a rotated playlist.

use ip_tv_neon::models::{
    AppData, CacheMeta, Channel, Config, ENTRY_SCHEMA, RadioStation, channel_key, radio_key,
};

fn channel(name: &str, url: &str, tvg_id: Option<&str>, norm_name: &str) -> Channel {
    Channel {
        name: name.into(),
        group: "Test".into(),
        url: url.into(),
        tvg_id: tvg_id.map(str::to_string),
        norm_name: norm_name.into(),
        catchup_days: 0,
        name_lower: name.to_lowercase(),
    }
}

fn radio(id: &str, title: &str, stream: &str) -> RadioStation {
    RadioStation {
        id: id.into(),
        title: title.into(),
        stream: stream.into(),
        quality_urls: Default::default(),
        genres: vec!["ALL".into()],
        provider: "test".into(),
        track: None,
    }
}

#[test]
fn channel_key_prefers_tvg_id_then_name_then_url() {
    assert_eq!(
        channel_key(&channel("A", "http://h/1", Some("Ch1"), "a")),
        "id:Ch1"
    );
    assert_eq!(
        channel_key(&channel("B", "http://h/2", None, "bee")),
        "name:bee"
    );
    assert_eq!(
        channel_key(&channel("C", "http://h/3", None, "")),
        "url:http://h/3"
    );
    assert_eq!(radio_key(&radio("42", "R", "http://r/1")), "radio:42");
}

#[test]
fn legacy_url_entries_are_migrated_to_stable_keys() {
    let old_url = "http://provider-a/iptv/TOKEN/1/index.m3u8";
    let old_radio = "http://radio.example/stream.aac";

    let mut data = AppData {
        channels: vec![
            channel("Channel One", old_url, Some("ch1"), "channelone"),
            channel(
                "Channel Two",
                "http://provider-a/iptv/TOKEN/2/index.m3u8",
                None,
                "ch2",
            ),
        ],
        radio: vec![radio("7", "Radio Seven", old_radio)],
        ..Default::default()
    };
    data.build_indices();

    // entry_schema 0 == as written by versions before stable keys.
    let mut cfg = Config {
        entry_schema: 0,
        ..Config::default()
    };
    cfg.favorites.insert(old_url.to_string());
    cfg.favorites.insert(old_radio.to_string());
    cfg.history = vec![old_url.to_string(), old_radio.to_string()];
    cfg.channel_names
        .insert(old_url.to_string(), "Channel One".to_string());

    assert!(cfg.migrate_entries(&data));
    assert_eq!(cfg.entry_schema, ENTRY_SCHEMA);

    assert!(cfg.favorites.contains("id:ch1"));
    assert!(cfg.favorites.contains("radio:7"));
    assert!(!cfg.favorites.iter().any(|k| k.starts_with("url:")));

    assert_eq!(cfg.history, vec!["id:ch1", "radio:7"]);
    assert_eq!(cfg.entry_urls.get("id:ch1").unwrap(), old_url);
    assert_eq!(cfg.entry_urls.get("radio:7").unwrap(), old_radio);
    assert_eq!(cfg.channel_names.get("id:ch1").unwrap(), "Channel One");

    // Idempotent: a second run on an already-migrated config changes nothing.
    assert!(!cfg.migrate_entries(&data));
}

#[test]
fn favorite_survives_a_playlist_link_rotation() {
    // Day 1: favorite saved while the provider served provider-a.
    let day1 = "http://provider-a/iptv/OLD/1/index.m3u8";
    let mut day1_cfg = Config {
        entry_schema: 0,
        ..Config::default()
    };
    day1_cfg.favorites.insert(day1.to_string());

    let mut day1_data = AppData {
        channels: vec![channel("Channel One", day1, Some("ch1"), "channelone")],
        ..Default::default()
    };
    day1_data.build_indices();
    day1_cfg.migrate_entries(&day1_data);
    assert!(day1_cfg.favorites.contains("id:ch1"));

    // Day 2: the provider moved to provider-b, so the playlist URLs differ,
    // but the tvg-id is the same — the favorite must still resolve.
    let mut day2_data = AppData {
        channels: vec![channel(
            "Channel One HD",
            "http://provider-b/iptv/NEW/1/index.m3u8",
            Some("ch1"),
            "channelone",
        )],
        ..Default::default()
    };
    day2_data.build_indices();

    let idx = day2_data.key_index.get("id:ch1").copied();
    assert_eq!(idx, Some(0));
    let resolved_url = &day2_data.channels[idx.unwrap()].url;
    assert_eq!(resolved_url, "http://provider-b/iptv/NEW/1/index.m3u8");
}

#[test]
fn unresolved_url_entry_is_rekeyed_once_the_playlist_returns() {
    // An entry saved while the channel was absent: no identity, so it stays a
    // `url:` key until a later update can resolve it.
    let url = "http://provider/iptv/1/index.m3u8";
    let mut cfg = Config::default();
    cfg.favorites.insert(format!("url:{url}"));
    cfg.entry_urls.insert(format!("url:{url}"), url.to_string());

    let empty = AppData::default();
    assert!(!cfg.rekey_entries(&empty));
    assert!(cfg.favorites.contains(&format!("url:{url}")));

    let mut data = AppData {
        channels: vec![channel("Now Present", url, Some("ch9"), "nowpresent")],
        ..Default::default()
    };
    data.build_indices();
    assert!(cfg.rekey_entries(&data));
    assert!(cfg.favorites.contains("id:ch9"));
    assert_eq!(cfg.entry_urls.get("id:ch9").unwrap(), url);
}

#[test]
fn url_entry_is_rekeyed_by_unambiguous_name_when_url_changed() {
    let old = "http://old-provider/iptv/1/index.m3u8";
    let mut cfg = Config::default();
    cfg.favorites.insert(format!("url:{old}"));
    cfg.entry_urls.insert(format!("url:{old}"), old.into());
    cfg.channel_names
        .insert(format!("url:{old}"), "Родное Кино".into());

    let mut data = AppData {
        channels: vec![
            channel(
                "Родное Кино",
                "http://new/9/index.m3u8",
                Some("9"),
                "родноекино",
            ),
            channel("Другое", "http://new/2/index.m3u8", Some("2"), "другое"),
        ],
        ..Default::default()
    };
    data.build_indices();

    assert!(cfg.rekey_entries(&data));
    assert!(cfg.favorites.contains("id:9"));
}

#[test]
fn ambiguous_name_is_not_rekeyed() {
    let old = "http://old-provider/iptv/1/index.m3u8";
    let mut cfg = Config::default();
    cfg.favorites.insert(format!("url:{old}"));
    cfg.entry_urls.insert(format!("url:{old}"), old.into());
    cfg.channel_names
        .insert(format!("url:{old}"), "Кинохит".into());

    let mut data = AppData {
        channels: vec![
            channel("Кинохит", "http://new/1/index.m3u8", Some("1"), "кинохит"),
            channel("Кинохит", "http://new/2/index.m3u8", Some("2"), "кинохит"),
        ],
        ..Default::default()
    };
    data.build_indices();

    assert!(!cfg.rekey_entries(&data));
    assert!(cfg.favorites.contains(&format!("url:{old}")));
}

#[test]
fn cache_meta_roundtrips() {
    let meta = CacheMeta {
        playlist_url: "http://127.0.0.1:8095/neon.m3u".into(),
        epg_url: "http://epg.one/epg2.xml.gz".into(),
        fetched_at: 1_700_000_000,
        channels: 2040,
        epg_channels: 2040,
        epg_warning: Some("EPG: HTTP 404 — guide unavailable".into()),
    };
    let json = serde_json::to_string(&meta).unwrap();
    let restored: CacheMeta = serde_json::from_str(&json).unwrap();
    assert_eq!(meta, restored);
}
