/// Which column is sorted
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortColumn {
    #[default]
    Name,
    Path,
    Size,
    Extension,
    DateModified,
    Type,
}

/// Sort direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    #[default]
    Ascending,
    Descending,
}

impl SortOrder {
    pub fn indicator(&self) -> &'static str {
        match self {
            SortOrder::Ascending => " \u{25B2}",
            SortOrder::Descending => " \u{25BC}",
        }
    }
}

/// Table display state
pub struct TableState {
    pub selected: Option<usize>,
    pub scroll_offset: usize,
    pub visible_rows: usize,
    pub sort_column: SortColumn,
    pub sort_order: SortOrder,
}

impl Default for TableState {
    fn default() -> Self {
        Self {
            selected: None,
            scroll_offset: 0,
            visible_rows: 20,
            sort_column: SortColumn::Name,
            sort_order: SortOrder::Ascending,
        }
    }
}

impl TableState {
    pub fn select_next(&mut self, total: usize) {
        if total == 0 {
            return;
        }
        let i = match self.selected {
            Some(i) => (i + 1).min(total - 1),
            None => 0,
        };
        self.selected = Some(i);
        self.ensure_visible(i);
    }

    pub fn select_prev(&mut self) {
        let i = match self.selected {
            Some(0) | None => 0,
            Some(i) => i - 1,
        };
        self.selected = Some(i);
        self.ensure_visible(i);
    }

    pub fn page_down(&mut self, total: usize) {
        if total == 0 {
            return;
        }
        let jump = self.visible_rows.saturating_sub(1);
        let i = match self.selected {
            Some(i) => (i + jump).min(total - 1),
            None => jump.min(total - 1),
        };
        self.selected = Some(i);
        self.ensure_visible(i);
    }

    pub fn page_up(&mut self) {
        let jump = self.visible_rows.saturating_sub(1);
        let i = match self.selected {
            Some(i) => i.saturating_sub(jump),
            None => 0,
        };
        self.selected = Some(i);
        self.ensure_visible(i);
    }

    pub fn select_first(&mut self) {
        self.selected = Some(0);
        self.scroll_offset = 0;
    }

    pub fn select_last(&mut self, total: usize) {
        if total == 0 {
            return;
        }
        self.selected = Some(total - 1);
        self.ensure_visible(total - 1);
    }

    fn ensure_visible(&mut self, index: usize) {
        if index < self.scroll_offset {
            self.scroll_offset = index;
        } else if self.visible_rows > 0 && index >= self.scroll_offset + self.visible_rows {
            self.scroll_offset = index - self.visible_rows + 1;
        }
    }
}
