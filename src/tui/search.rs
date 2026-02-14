/// Search input state for the TUI
pub struct SearchState {
    pub query: String,
    pub cursor_pos: usize,
    pub focused: bool,
    pub needs_search: bool,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            query: String::new(),
            cursor_pos: 0,
            focused: true,
            needs_search: false,
        }
    }
}

/// Check if a filename matches a pattern.
/// Supports `*` wildcards: `*.ext`, `prefix*`, `*text*`, or plain substring.
pub fn matches_pattern(name: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }

    let has_leading_star = pattern.starts_with('*');
    let has_trailing_star = pattern.ends_with('*');

    match (has_leading_star, has_trailing_star) {
        (true, true) if pattern.len() > 2 => {
            // *text* -> contains
            name.contains(&pattern[1..pattern.len() - 1])
        }
        (true, true) => {
            // Just "*" or "**" -> matches everything
            true
        }
        (true, false) => {
            // *.ext -> ends with
            name.ends_with(&pattern[1..])
        }
        (false, true) => {
            // prefix* -> starts with
            name.starts_with(&pattern[..pattern.len() - 1])
        }
        (false, false) => {
            // plain substring match
            name.contains(pattern)
        }
    }
}
