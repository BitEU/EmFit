//! Search state management

/// Search state
#[derive(Default)]
pub struct SearchState {
    /// Current search query
    pub query: String,
    /// Whether a search is needed
    pub needs_search: bool,
    /// First frame flag (for auto-focus)
    pub first_frame: bool,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            needs_search: false,
            first_frame: true,
        }
    }
}
