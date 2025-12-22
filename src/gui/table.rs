//! Results table state

use crate::SearchResult;

/// Column to sort by
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortColumn {
    #[default]
    Name,
    Path,
    Size,
    DateModified,
}

/// Sort order
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    #[default]
    Ascending,
    Descending,
}

impl SortOrder {
    pub fn toggle(&mut self) {
        *self = match self {
            SortOrder::Ascending => SortOrder::Descending,
            SortOrder::Descending => SortOrder::Ascending,
        };
    }

    pub fn indicator(&self) -> &'static str {
        match self {
            SortOrder::Ascending => " \u{25B2}",  // Up triangle
            SortOrder::Descending => " \u{25BC}", // Down triangle
        }
    }
}

/// Results table state
#[derive(Default)]
pub struct ResultsTable {
    /// Search results (kept for compatibility but mostly unused now)
    pub results: Vec<SearchResult>,
    /// Currently selected row index in the filtered view
    pub selected: Option<usize>,
    /// Sort column
    pub sort_column: SortColumn,
    /// Sort order
    pub sort_order: SortOrder,
}

impl ResultsTable {
    /// Clear all results
    pub fn clear(&mut self) {
        self.results.clear();
        self.selected = None;
    }

    /// Add a search result (for compatibility)
    pub fn add_result(&mut self, result: SearchResult) {
        self.results.push(result);
    }

    /// Get selected result (for compatibility)
    pub fn get_selected(&self) -> Option<&SearchResult> {
        self.selected.and_then(|idx| self.results.get(idx))
    }

    /// Sort results by current column and order (for compatibility)
    pub fn sort(&mut self) {
        let order = self.sort_order;
        match self.sort_column {
            SortColumn::Name => {
                self.results.sort_by(|a, b| {
                    let cmp = a.name.to_lowercase().cmp(&b.name.to_lowercase());
                    if order == SortOrder::Descending {
                        cmp.reverse()
                    } else {
                        cmp
                    }
                });
            }
            SortColumn::Path => {
                self.results.sort_by(|a, b| {
                    let cmp = a.path.to_lowercase().cmp(&b.path.to_lowercase());
                    if order == SortOrder::Descending {
                        cmp.reverse()
                    } else {
                        cmp
                    }
                });
            }
            SortColumn::Size => {
                self.results.sort_by(|a, b| {
                    let cmp = a.file_size.cmp(&b.file_size);
                    if order == SortOrder::Descending {
                        cmp.reverse()
                    } else {
                        cmp
                    }
                });
            }
            SortColumn::DateModified => {
                self.results.sort_by(|a, b| {
                    let cmp = a.modification_time.cmp(&b.modification_time);
                    if order == SortOrder::Descending {
                        cmp.reverse()
                    } else {
                        cmp
                    }
                });
            }
        }
    }
}
