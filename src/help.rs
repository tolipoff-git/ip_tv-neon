// In-app key reference.
//
// Why: every screen has its own keys and they were only discoverable by
// reading the README or the code. This module is the single table behind the
// Help screen (Main Menu → ❓ Help, or F1 / ? anywhere), so adding a screen
// means adding one row here instead of hunting through handlers.

pub struct HelpSection {
    pub title: &'static str,
    /// (key, what it does)
    pub rows: &'static [(&'static str, &'static str)],
}

pub static HELP: &[HelpSection] = &[
    HelpSection {
        title: "GENERAL",
        rows: &[
            ("F1  ·  ?", "open / close this help"),
            ("Ctrl+C", "quit the app from anywhere"),
            ("Esc", "back one screen (quits on the Main Menu)"),
        ],
    },
    HelpSection {
        title: "MAIN MENU",
        rows: &[
            ("↑ ↓", "move between items"),
            ("Enter", "open the selected item"),
        ],
    },
    HelpSection {
        title: "IPTV — CATEGORIES & CHANNELS",
        rows: &[
            ("↑ ↓", "move"),
            ("Enter", "open category / open the channel's detail (EPG)"),
            (
                "F  ·  f",
                "favourite the channel ('f' works while the search box is empty)",
            ),
            ("type letters", "filter channels by name"),
            ("Backspace", "erase one character of the filter"),
            ("Esc", "clear the filter, then back"),
        ],
    },
    HelpSection {
        title: "CHANNEL DETAIL (EPG)",
        rows: &[
            ("↑ ↓", "select a programme"),
            (
                "Enter",
                "play the selected programme (past + archive → catch-up URL)",
            ),
            ("l", "play the live stream"),
            ("f", "toggle favourite"),
            ("Esc", "back"),
        ],
    },
    HelpSection {
        title: "RADIO",
        rows: &[
            ("↑ ↓", "move"),
            ("Enter", "play the selected station"),
            ("f", "toggle favourite"),
            ("Esc", "back"),
            ("+  ·  -", "volume up / down while playing"),
            ("m", "mute / unmute"),
            ("Space", "pause / resume"),
        ],
    },
    HelpSection {
        title: "WHILE TV PLAYS (mpv window)",
        rows: &[
            ("↑ ↓", "switch to the previous / next channel"),
            ("Esc", "stop playback"),
        ],
    },
    HelpSection {
        title: "FAVOURITES  ·  HISTORY",
        rows: &[
            ("↑ ↓", "move (↑ ↓ also switches channels while TV plays)"),
            ("Enter", "play the entry"),
            ("Esc", "back"),
        ],
    },
    HelpSection {
        title: "AI CHAT",
        rows: &[
            ("type", "write your question"),
            ("Enter", "send"),
            ("Tab", "move focus to the result list"),
            ("↑ ↓", "select a result"),
            ("Enter", "play the selected result"),
            ("d", "open its channel detail"),
            ("Esc", "back"),
        ],
    },
    HelpSection {
        title: "LOCAL PLAYLISTS",
        rows: &[("Esc", "back")],
    },
    HelpSection {
        title: "SETTINGS",
        rows: &[
            ("↑ ↓", "move between rows"),
            ("← →", "change a toggle / cycle (theme, AI provider)"),
            (
                "Enter",
                "edit a text row, toggle a switch, or run a row's action",
            ),
            ("T", "test the configured playlist and EPG (HTTP status)"),
            ("R", "reset the selected text row to its default"),
            ("Esc", "back"),
        ],
    },
    HelpSection {
        title: "SETTINGS — EDITING A VALUE",
        rows: &[
            ("← →  Home  End", "move the cursor"),
            ("Backspace  Delete", "delete a character"),
            ("Ctrl+U", "clear the whole line"),
            ("Ctrl+W", "delete the previous word"),
            (
                "paste",
                "replaces the whole value in a freshly opened field",
            ),
            ("Ctrl+T", "test the value being edited before saving"),
            (
                "Enter",
                "save (invalid input stays in the editor with the reason)",
            ),
            ("Esc", "cancel"),
        ],
    },
    HelpSection {
        title: "CONFIRMATION DIALOGS",
        rows: &[
            ("Y  ·  Enter", "confirm (clear list, download now)"),
            ("N  ·  Esc", "cancel"),
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::HELP;
    use std::collections::HashSet;

    #[test]
    fn every_section_has_rows_and_a_unique_title() {
        assert!(!HELP.is_empty());
        let mut titles = HashSet::new();
        for section in HELP {
            assert!(
                titles.insert(section.title),
                "duplicate help section {:?}",
                section.title
            );
            assert!(!section.rows.is_empty(), "empty help section");
            for (key, action) in section.rows {
                assert!(!key.is_empty() && !action.is_empty());
            }
        }
    }

    #[test]
    fn help_is_longer_than_a_screen_so_scrolling_is_exercised() {
        let rows: usize = HELP.iter().map(|s| s.rows.len()).sum();
        assert!(rows >= 20, "expected a substantial key reference");
    }
}
