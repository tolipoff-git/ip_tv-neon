// The Settings "test sources" probe must judge a source without downloading it
// whole — these cover the local-file and rejection paths.

use ip_tv_neon::epg::{SourceKind, probe_source};

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(ip_tv_neon::consts::UA)
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap()
}

#[tokio::test]
async fn local_playlist_file_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mini.m3u");
    std::fs::write(&path, "#EXTM3U\n#EXTINF:-1,T\nhttp://h/s\n").unwrap();

    let msg = probe_source(&client(), path.to_str().unwrap(), SourceKind::Playlist)
        .await
        .expect("local m3u must probe OK");
    assert!(msg.starts_with("OK — local file"), "{msg}");
}

#[tokio::test]
async fn html_file_is_rejected_as_playlist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-a-playlist.m3u");
    std::fs::write(&path, "<html><body>nope</body></html>").unwrap();

    let err = probe_source(&client(), path.to_str().unwrap(), SourceKind::Playlist)
        .await
        .expect_err("HTML must not pass as a playlist");
    assert!(err.contains("M3U"), "{err}");
}

#[tokio::test]
async fn epg_must_be_http() {
    let err = probe_source(&client(), "/tmp/epg.xml", SourceKind::Epg)
        .await
        .expect_err("local EPG path is unsupported");
    assert!(err.contains("http"), "{err}");
}

#[tokio::test]
async fn empty_source_is_reported() {
    let err = probe_source(&client(), "   ", SourceKind::Playlist)
        .await
        .expect_err("blank source");
    assert_eq!(err, "empty");
}
