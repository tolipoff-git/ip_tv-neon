// Single-line text editor used by the Settings screen.
//
// Why: the previous editor was a bare `String` with only "append char" and
// "pop last char". Replacing a long playlist URL therefore meant one Backspace
// per character, with no cursor, no clear, and no way to paste a new value
// without gluing it to the old one. This type keeps a cursor and implements the
// editing operations users expect from any input field, so the UI layer only
// has to draw it.

/// Cursor-aware single-line editor state.
///
/// `fresh` is true until the first edit after opening. While fresh, a paste
/// replaces the whole pre-filled value (the common "swap this link" gesture);
/// after any edit the paste is inserted at the cursor like normal typing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextInput {
    chars: Vec<char>,
    cursor: usize,
    fresh: bool,
}

impl TextInput {
    /// Start editing `value` with the cursor at the end.
    pub fn new(value: &str) -> Self {
        let chars: Vec<char> = value.chars().collect();
        let cursor = chars.len();
        Self {
            chars,
            cursor,
            fresh: true,
        }
    }

    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// True while the field still holds its pre-filled value untouched — a
    /// paste in this state replaces the whole value.
    pub fn is_fresh(&self) -> bool {
        self.fresh
    }

    pub fn insert_char(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
        self.fresh = false;
    }

    /// Insert pasted text. Control characters are dropped so a pasted payload
    /// cannot inject newlines or terminal escapes into a single-line field.
    pub fn insert_str(&mut self, text: &str) {
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        if clean.is_empty() {
            return;
        }
        if self.fresh {
            self.chars.clear();
            self.cursor = 0;
        }
        for c in clean.chars() {
            self.chars.insert(self.cursor, c);
            self.cursor += 1;
        }
        self.fresh = false;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
        self.fresh = false;
    }

    pub fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
        self.fresh = false;
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
        self.fresh = false;
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.chars.len() {
            self.cursor += 1;
        }
        self.fresh = false;
    }

    pub fn home(&mut self) {
        self.cursor = 0;
        self.fresh = false;
    }

    pub fn end(&mut self) {
        self.cursor = self.chars.len();
        self.fresh = false;
    }

    /// Delete everything before the cursor (Ctrl+U) — the fastest way to clear
    /// a long URL before pasting a new one.
    pub fn kill_to_start(&mut self) {
        self.chars.drain(..self.cursor);
        self.cursor = 0;
        self.fresh = false;
    }

    /// Delete the word before the cursor plus any trailing whitespace (Ctrl+W).
    pub fn kill_word(&mut self) {
        let mut start = self.cursor;
        while start > 0 && self.chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !self.chars[start - 1].is_whitespace() {
            start -= 1;
        }
        self.chars.drain(start..self.cursor);
        self.cursor = start;
        self.fresh = false;
    }
}

#[cfg(test)]
mod tests {
    use super::TextInput;

    #[test]
    fn cursor_starts_at_end() {
        let input = TextInput::new("abc");
        assert_eq!(input.cursor(), 3);
        assert_eq!(input.value(), "abc");
        assert!(input.is_fresh());
    }

    #[test]
    fn paste_replaces_fresh_value_but_appends_after_edit() {
        let mut input = TextInput::new("http://old/long/url.m3u");
        input.insert_str("http://new");
        assert_eq!(input.value(), "http://new");

        let mut edited = TextInput::new("http://old");
        edited.end();
        edited.backspace();
        edited.insert_str("x");
        assert_eq!(edited.value(), "http://olx");
    }

    #[test]
    fn arrows_home_end_and_delete_work() {
        let mut input = TextInput::new("abcd");
        input.home();
        input.delete();
        assert_eq!(input.value(), "bcd");
        input.end();
        input.move_left();
        input.insert_char('X');
        assert_eq!(input.value(), "bcXd");
        input.home();
        input.move_right();
        assert_eq!(input.cursor(), 1);
    }

    #[test]
    fn kill_to_start_clears_prefix_and_kill_word_drops_word() {
        let mut input = TextInput::new("http://host/path");
        input.kill_to_start();
        assert_eq!(input.value(), "");

        let mut words = TextInput::new("one two three");
        words.kill_word();
        assert_eq!(words.value(), "one two ");
        words.kill_word();
        assert_eq!(words.value(), "one ");
    }

    #[test]
    fn control_chars_are_ignored() {
        let mut input = TextInput::new("");
        input.insert_str("http://a\n\tb\x1b[31m");
        assert_eq!(input.value(), "http://ab[31m");
    }
}
