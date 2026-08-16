//! The one-line editor behind the query field of the interactive mode.
//!
//! Kept apart from the drawing code so the movements and deletions can be
//! tested without a terminal.
//!
//! Positions are counted in characters rather than display columns, like the
//! rest of the UI: a wide CJK glyph or an emoji still counts as one. Byte
//! offsets appear only where `String` demands them — slicing a query on bytes
//! would panic mid-character, and queries arrive in every language.

/// A query being typed: the text, the cursor, and the window shown on screen.
#[derive(Default)]
pub struct LineEditor {
    text: String,
    /// Cursor position, in characters from the start.
    cursor: usize,
    /// Leftmost character of the window drawn in the input box.
    offset: usize,
}

impl LineEditor {
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Nothing but whitespace: not worth sending to the model.
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.offset = 0;
    }

    pub fn insert_char(&mut self, c: char) {
        let at = self.byte_index(self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let at = self.byte_index(self.cursor);
        self.text.remove(at);
    }

    pub fn delete(&mut self) {
        if self.cursor < self.len() {
            let at = self.byte_index(self.cursor);
            self.text.remove(at);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.len();
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start();
    }

    pub fn word_right(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor;
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        self.cursor = i;
    }

    pub fn delete_word_back(&mut self) {
        let start = self.word_start();
        let range = self.byte_index(start)..self.byte_index(self.cursor);
        self.text.replace_range(range, "");
        self.cursor = start;
    }

    pub fn kill_to_start(&mut self) {
        let range = ..self.byte_index(self.cursor);
        self.text.replace_range(range, "");
        self.cursor = 0;
    }

    pub fn kill_to_end(&mut self) {
        let at = self.byte_index(self.cursor);
        self.text.truncate(at);
    }

    /// The slice of the query that fits in the input box, and the cursor's
    /// column within it.
    ///
    /// The window follows the cursor in both directions: it scrolls left once
    /// the cursor walks off the left edge and right once it passes the right
    /// one. The offset is remembered rather than recomputed from the cursor,
    /// or the text would jump around under a cursor sitting still. One column
    /// is held back for the cursor itself, or it would sit on the border.
    pub fn visible(&mut self, inner_width: u16) -> (String, u16) {
        let room = (inner_width as usize).saturating_sub(1);
        if room == 0 {
            self.offset = 0;
            return (String::new(), 0);
        }

        // Keep the window inside the text first: after a deletion the old
        // offset can point past what is left.
        self.offset = self.offset.min(self.len().saturating_sub(room));
        if self.cursor < self.offset {
            self.offset = self.cursor;
        }
        if self.cursor > self.offset + room {
            self.offset = self.cursor - room;
        }

        let visible = self.text.chars().skip(self.offset).take(room).collect();
        (visible, (self.cursor - self.offset) as u16)
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    /// Byte offset of the character at `index`, or the end of the text.
    fn byte_index(&self, index: usize) -> usize {
        self.text
            .char_indices()
            .nth(index)
            .map_or(self.text.len(), |(at, _)| at)
    }

    /// Where the word to the left of the cursor begins: skip the whitespace
    /// between the two, then the word itself — the same boundary `Ctrl+W` uses
    /// in a shell.
    fn word_start(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An editor holding `text` with the cursor at its end, as if typed.
    fn editor(text: &str) -> LineEditor {
        let mut editor = LineEditor::default();
        for c in text.chars() {
            editor.insert_char(c);
        }
        editor
    }

    #[test]
    fn typing_goes_in_at_the_cursor() {
        let mut editor = editor("find files");
        editor.home();
        editor.insert_char('!');
        assert_eq!(editor.text(), "!find files");
        editor.end();
        editor.insert_char('?');
        assert_eq!(editor.text(), "!find files?");
    }

    #[test]
    fn backspace_and_delete_work_on_both_sides_of_the_cursor() {
        let mut editor = editor("abcd");
        editor.left();
        editor.left();
        editor.backspace();
        assert_eq!(editor.text(), "acd");
        editor.delete();
        assert_eq!(editor.text(), "ad");
    }

    #[test]
    fn the_cursor_stops_at_both_ends() {
        let mut editor = editor("ab");
        for _ in 0..5 {
            editor.left();
        }
        editor.backspace();
        assert_eq!(
            editor.text(),
            "ab",
            "backspace at the start deletes nothing"
        );
        for _ in 0..5 {
            editor.right();
        }
        editor.delete();
        assert_eq!(editor.text(), "ab", "delete at the end deletes nothing");
    }

    #[test]
    fn words_are_bounded_by_whitespace() {
        let mut editor = editor("покажи занятое место");
        editor.word_left();
        assert_eq!(editor.cursor, 15, "start of «место»");
        editor.word_left();
        assert_eq!(editor.cursor, 7, "start of «занятое»");
        editor.word_right();
        assert_eq!(editor.cursor, 14, "end of «занятое»");
    }

    #[test]
    fn deleting_a_word_takes_the_space_before_it() {
        let mut editor = editor("find the largest file");
        editor.delete_word_back();
        assert_eq!(editor.text(), "find the largest ");
        editor.delete_word_back();
        assert_eq!(editor.text(), "find the ");
    }

    #[test]
    fn killing_to_the_start_leaves_the_tail() {
        let mut line = editor("занятое место");
        line.word_left();
        line.kill_to_start();
        assert_eq!(line.text(), "место");
        assert_eq!(line.cursor, 0);
    }

    #[test]
    fn killing_to_the_end_leaves_the_head() {
        let mut line = editor("занятое место");
        line.word_left();
        line.kill_to_end();
        assert_eq!(line.text(), "занятое ");
    }

    #[test]
    fn a_short_query_is_shown_whole() {
        let (visible, cursor) = editor("find big files").visible(40);
        assert_eq!(visible, "find big files");
        assert_eq!(cursor, 14);
    }

    #[test]
    fn a_long_query_scrolls_to_keep_the_end_in_view() {
        // Typing past the right edge used to leave the user writing blind.
        let (visible, cursor) = editor("abcdefghijklmnopqrstuvwxyz").visible(10);
        assert_eq!(visible, "rstuvwxyz");
        assert_eq!(
            visible.chars().count(),
            9,
            "one column is left for the cursor"
        );
        assert_eq!(cursor, 9);
    }

    #[test]
    fn the_window_follows_the_cursor_back_to_the_start() {
        let mut editor = editor("abcdefghijklmnopqrstuvwxyz");
        editor.visible(10);
        editor.home();
        let (visible, cursor) = editor.visible(10);
        assert_eq!(visible, "abcdefghi");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn the_window_stays_put_while_the_cursor_moves_inside_it() {
        let mut editor = editor("abcdefghijklmnopqrstuvwxyz");
        let (before, _) = editor.visible(10);
        editor.left();
        let (after, cursor) = editor.visible(10);
        assert_eq!(before, after, "one step left must not scroll the text");
        assert_eq!(cursor, 8);
    }

    #[test]
    fn the_cursor_never_leaves_the_box() {
        for width in 0..40u16 {
            let mut editor = editor("a rather long query indeed");
            editor.word_left();
            let (visible, cursor) = editor.visible(width);
            assert!(cursor <= width, "cursor {cursor} past width {width}");
            assert!(visible.chars().count() <= width as usize);
        }
    }

    #[test]
    fn scrolling_counts_characters_not_bytes() {
        // Slicing these on bytes would both cut too much and panic mid-character.
        let (visible, cursor) = editor("покажи занятое место на диске").visible(10);
        assert_eq!(visible, " на диске");
        assert_eq!(cursor, 9);
    }

    #[test]
    fn a_query_of_spaces_is_blank() {
        assert!(editor("   ").is_blank());
        assert!(!editor(" go ").is_blank());
    }
}
