//! Scroll/selection model for a channel list panel.
//!
//! Front-ends render it however they like (mpv ASS overlay on desktop, a
//! native list on Android); this only tracks which channels exist in the
//! panel, which one is selected and which window of rows is visible.

#[derive(Debug, Clone)]
pub struct Browser {
    /// Channel indices shown in the panel, ascending.
    items: Vec<usize>,
    /// Position in `items` of the selected entry.
    selected: usize,
    /// Position in `items` of the first visible row.
    scroll: usize,
    rows: usize,
}

impl Browser {
    /// `items` must be sorted ascending (as returned by `Tuner::view`).
    pub fn new(items: Vec<usize>, focus: usize, rows: usize) -> Self {
        let mut b = Self {
            items,
            selected: 0,
            scroll: 0,
            rows: rows.max(1),
        };
        b.focus(focus, true);
        b
    }

    /// Replaces the contents (e.g. after switching group), keeping `focus`
    /// (or the nearest following channel) selected and centered.
    pub fn set_items(&mut self, items: Vec<usize>, focus: usize) {
        self.items = items;
        self.selected = 0;
        self.scroll = 0;
        self.focus(focus, true);
    }

    /// Replaces the contents while keeping the view steady: the selected
    /// channel (or the nearest following one) stays on the same screen row.
    /// Used when channels disappear while the user is scrolling.
    pub fn replace_items(&mut self, items: Vec<usize>) {
        let Some(selected) = self.selected_channel() else {
            self.items = items;
            return;
        };
        let row = self.selected.saturating_sub(self.scroll);
        self.items = items;
        if self.items.is_empty() {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        self.selected = self.position(selected);
        self.scroll = self.selected.saturating_sub(row).min(self.max_scroll());
    }

    pub fn items(&self) -> &[usize] {
        &self.items
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn scroll(&self) -> usize {
        self.scroll
    }
    pub fn selected_channel(&self) -> Option<usize> {
        self.items.get(self.selected).copied()
    }

    /// Where `channel` is in the list, or (in a list sorted by number) the
    /// nearest following one. Lists aren't always sorted (recently watched).
    fn position(&self, channel: usize) -> usize {
        self.items
            .iter()
            .position(|&c| c == channel)
            .unwrap_or_else(|| self.items.partition_point(|&c| c < channel))
            .min(self.items.len().saturating_sub(1))
    }

    fn max_scroll(&self) -> usize {
        self.items.len().saturating_sub(self.rows)
    }

    /// Selects `channel` (or the nearest following one if it is not in the
    /// list). Centers it if `center` is set or it is off screen.
    pub fn focus(&mut self, channel: usize, center: bool) {
        if self.items.is_empty() {
            return;
        }
        let pos = self.position(channel);
        self.selected = pos;
        if center || !self.is_visible(pos) {
            self.scroll = pos.saturating_sub(self.rows / 2).min(self.max_scroll());
        }
    }

    fn is_visible(&self, pos: usize) -> bool {
        pos >= self.scroll && pos < self.scroll + self.rows
    }

    /// Moves the selection, scrolling just enough to keep it visible.
    pub fn move_by(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let last = self.items.len() as isize - 1;
        self.selected = (self.selected as isize)
            .saturating_add(delta)
            .clamp(0, last) as usize;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + self.rows {
            self.scroll = self.selected + 1 - self.rows;
        }
    }

    pub fn page(&mut self, pages: isize) {
        self.move_by(pages.saturating_mul(self.rows as isize));
    }
    pub fn home(&mut self) {
        self.move_by(isize::MIN / 2);
    }
    pub fn end(&mut self) {
        self.move_by(isize::MAX / 2);
    }

    /// Scrolls the view without moving the selection (mouse wheel).
    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = (self.scroll as isize)
            .saturating_add(delta)
            .clamp(0, self.max_scroll() as isize) as usize;
    }

    /// Jumps the view to a fraction (0..=1) of the list (scrollbar click).
    pub fn scroll_to_fraction(&mut self, f: f64) {
        self.scroll = (f.clamp(0.0, 1.0) * self.max_scroll() as f64).round() as usize;
    }

    /// Selects the entry shown at visible `row`; returns its channel.
    pub fn select_row(&mut self, row: usize) -> Option<usize> {
        let pos = self.scroll + row;
        if row < self.rows && pos < self.items.len() {
            self.selected = pos;
            Some(self.items[pos])
        } else {
            None
        }
    }

    /// `(row, channel, is_selected)` for every visible row.
    pub fn visible(&self) -> impl Iterator<Item = (usize, usize, bool)> + '_ {
        self.items
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(self.rows)
            .map(move |(pos, &ch)| (pos - self.scroll, ch, pos == self.selected))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(b: &Browser) -> Vec<usize> {
        b.visible().map(|(_, ch, _)| ch).collect()
    }

    #[test]
    fn centers_on_focus_and_clamps_at_edges() {
        let b = Browser::new((0..100).collect(), 50, 10);
        assert_eq!(rows(&b), (45..55).collect::<Vec<_>>());
        assert_eq!(b.selected_channel(), Some(50));

        let b = Browser::new((0..100).collect(), 1, 10);
        assert_eq!(b.scroll(), 0);
        let b = Browser::new((0..100).collect(), 99, 10);
        assert_eq!(rows(&b), (90..100).collect::<Vec<_>>());
    }

    #[test]
    fn focus_missing_channel_picks_next() {
        let b = Browser::new(vec![2, 4, 8, 16], 5, 3);
        assert_eq!(b.selected_channel(), Some(8));
        let b = Browser::new(vec![2, 4, 8, 16], 99, 3);
        assert_eq!(b.selected_channel(), Some(16));
    }

    #[test]
    fn keyboard_keeps_selection_visible() {
        let mut b = Browser::new((0..100).collect(), 0, 10);
        b.move_by(12);
        assert_eq!(b.selected_channel(), Some(12));
        assert_eq!(b.scroll(), 3);
        b.move_by(-1000);
        assert_eq!((b.selected_channel(), b.scroll()), (Some(0), 0));
        b.page(1);
        assert_eq!(b.selected_channel(), Some(10));
        b.end();
        assert_eq!((b.selected_channel(), b.scroll()), (Some(99), 90));
        b.home();
        assert_eq!(b.selected_channel(), Some(0));
    }

    #[test]
    fn wheel_scrolls_without_moving_selection() {
        let mut b = Browser::new((0..100).collect(), 0, 10);
        b.scroll_by(3);
        assert_eq!((b.scroll(), b.selected_channel()), (3, Some(0)));
        b.scroll_by(1000);
        assert_eq!(b.scroll(), 90);
        b.scroll_by(-1000);
        assert_eq!(b.scroll(), 0);
        b.scroll_to_fraction(0.5);
        assert_eq!(b.scroll(), 45);
    }

    #[test]
    fn replace_items_keeps_view_steady() {
        let mut b = Browser::new((0..100).collect(), 50, 10); // rows 45..55
        b.move_by(2); // select 52 on row 7
                      // Remove every channel below 40 and channel 52 itself.
        b.replace_items((40..100).filter(|&c| c != 52).collect());
        assert_eq!(b.selected_channel(), Some(53));
        let row = b.visible().find(|&(_, _, sel)| sel).map(|(r, _, _)| r);
        assert_eq!(row, Some(7));

        b.replace_items(vec![]);
        assert_eq!(b.selected_channel(), None);
    }

    #[test]
    fn select_row_and_short_lists() {
        let mut b = Browser::new(vec![3, 7, 9], 3, 10);
        assert_eq!(b.select_row(2), Some(9));
        assert_eq!(b.select_row(3), None);
        b.scroll_by(5);
        assert_eq!(b.scroll(), 0);
        let mut empty = Browser::new(vec![], 0, 10);
        empty.move_by(1);
        assert_eq!(empty.selected_channel(), None);
    }
}
