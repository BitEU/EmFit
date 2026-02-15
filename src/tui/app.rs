use crate::file_tree::NodeKey;
use crate::tui::colors;
use crate::tui::menu::{
    ActionKind, ActiveMenu, ActionsMenu, ConfirmDialog, RenameDialog, SearchFiltersMenu,
    SearchFilterField,
};
use crate::tui::search::{matches_pattern, SearchState};
use crate::tui::table::{SortColumn, SortOrder, TableState};
use crate::tui::ui;
use crate::{FileTree, MultiVolumeScanner, ScanConfig, VolumeScanner};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::prelude::*;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Messages from background threads
pub enum BgMessage {
    ScanProgress(String),
    ScanComplete(Arc<FileTree>),
    ScanError(String),
    SortComplete(SortColumn, Vec<usize>),
    MetadataRefreshComplete(Vec<(usize, u64, u64)>),
    PathCacheComplete(Vec<(usize, String)>),
}

/// Lightweight cached entry for fast search/sort without touching the tree
#[derive(Clone)]
pub struct EntryData {
    pub tree_index: usize,
    pub key: NodeKey,
    pub file_reference_number: u64,
    pub name: String,
    pub name_lower: String,
    pub extension: String,
    pub file_size: u64,
    pub modification_time: u64,
    pub is_directory: bool,
    pub cached_path: String,
    pub path_lower: String,
}

/// Row data extracted for rendering (only built for visible rows)
pub struct RowData {
    pub name: String,
    pub path: String,
    pub file_size: u64,
    pub is_directory: bool,
    pub modification_time: u64,
    pub extension: String,
}

pub struct App {
    // Data
    pub trees: Vec<Arc<FileTree>>,
    pub all_entries: Vec<EntryData>,
    pub filtered_indices: Vec<usize>,

    // Sub-states
    pub search: SearchState,
    pub table: TableState,

    // Scanning state
    pub is_scanning: bool,
    pub is_sorting: bool,
    pub is_refreshing_metadata: bool,
    pub scan_progress: String,
    pub status_message: String,
    pub total_count: u64,

    // Drives
    pub selected_drives: Vec<char>,

    // Sort optimization
    last_sort_column: Option<SortColumn>,
    last_sort_order: SortOrder,

    // Channel
    bg_receiver: Option<Receiver<BgMessage>>,
    bg_sender: Option<Sender<BgMessage>>,

    // Metadata refresh tracking
    pending_metadata_refresh: std::collections::HashSet<usize>,

    // Active menu/dialog overlay
    pub active_menu: ActiveMenu,

    // Persistent search filters (applied even when menu is closed)
    pub search_filters: SearchFiltersMenu,

    // Quit flag
    pub should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        let available_drives = MultiVolumeScanner::detect_ntfs_volumes();
        let selected_drives = available_drives.clone();

        let mut app = Self {
            trees: Vec::new(),
            all_entries: Vec::new(),
            filtered_indices: Vec::new(),
            search: SearchState::default(),
            table: TableState::default(),
            is_scanning: false,
            is_sorting: false,
            is_refreshing_metadata: false,
            scan_progress: String::new(),
            status_message: "Ready".to_string(),
            total_count: 0,
            selected_drives,
            last_sort_column: None,
            last_sort_order: SortOrder::Ascending,
            bg_receiver: None,
            bg_sender: None,
            pending_metadata_refresh: std::collections::HashSet::new(),
            active_menu: ActiveMenu::None,
            search_filters: SearchFiltersMenu::new(),
            should_quit: false,
        };

        if !app.selected_drives.is_empty() {
            app.start_scan();
        }

        app
    }

    pub fn run(&mut self, terminal: &mut Terminal<impl Backend>) -> crate::Result<()> {
        let tick_rate = Duration::from_millis(50);
        let mut last_tick = Instant::now();

        loop {
            terminal.draw(|frame| ui::draw(frame, self))?;

            let timeout = tick_rate.saturating_sub(last_tick.elapsed());
            if event::poll(timeout).unwrap_or(false) {
                if let Ok(Event::Key(key)) = event::read() {
                    // Only handle key press events, ignore key release and repeat
                    if key.kind == KeyEventKind::Press {
                        self.handle_key(key);
                    }
                }
            }

            if last_tick.elapsed() >= tick_rate {
                self.process_messages();
                if self.search.needs_search && !self.is_scanning {
                    self.perform_search();
                    self.search.needs_search = false;
                }
                last_tick = Instant::now();
            }

            if self.should_quit {
                return Ok(());
            }
        }
    }

    fn start_scan(&mut self) {
        if self.is_scanning || self.selected_drives.is_empty() {
            return;
        }

        self.is_scanning = true;
        self.scan_progress = "Starting scan...".to_string();
        self.trees.clear();
        self.all_entries.clear();
        self.filtered_indices.clear();
        self.table.selected = None;
        self.total_count = 0;
        self.last_sort_column = None;

        let (tx, rx) = channel();
        self.bg_receiver = Some(rx);
        self.bg_sender = Some(tx.clone());

        let drives = self.selected_drives.clone();

        thread::spawn(move || {
            for drive in drives {
                let _ = tx.send(BgMessage::ScanProgress(format!("Scanning {}:...", drive)));

                let config = ScanConfig {
                    use_usn: true,
                    use_mft: true,
                    include_hidden: true,
                    include_system: true,
                    calculate_sizes: true,
                    show_progress: false, // Don't write progress bars to stdout
                    ..Default::default()
                };

                let mut scanner = VolumeScanner::new(drive).with_config(config);

                match scanner.scan() {
                    Ok(tree) => {
                        let _ = tx.send(BgMessage::ScanComplete(Arc::new(tree)));
                    }
                    Err(e) => {
                        let _ = tx.send(BgMessage::ScanError(format!(
                            "Error scanning {}: {}",
                            drive, e
                        )));
                    }
                }
            }
        });
    }

    fn process_messages(&mut self) {
        let rx = match &self.bg_receiver {
            Some(rx) => rx,
            None => return,
        };

        while let Ok(msg) = rx.try_recv() {
            match msg {
                BgMessage::ScanProgress(msg) => {
                    self.scan_progress = msg;
                }
                BgMessage::ScanComplete(tree) => {
                    let drive = tree.drive_letter;
                    let files = tree.stats.total_files;
                    let dirs = tree.stats.total_directories;

                    // Prevent duplicates
                    if self.trees.iter().any(|t| t.drive_letter == drive) {
                        continue;
                    }

                    // Build EntryData with cached sort keys
                    let tree_index = self.trees.len();
                    for entry in tree.iter() {
                        let key = *entry.key();
                        let node = entry.value();
                        if !node.name.is_empty() {
                            let extension = extract_extension(&node.name);
                            self.all_entries.push(EntryData {
                                tree_index,
                                key,
                                file_reference_number: node.file_reference_number,
                                name: node.name.clone(),
                                name_lower: node.name.to_lowercase(),
                                extension,
                                file_size: node.file_size,
                                modification_time: node.modification_time,
                                is_directory: node.is_directory,
                                cached_path: String::new(),
                                path_lower: String::new(),
                            });
                        }
                    }

                    self.trees.push(tree);
                    self.total_count += files + dirs;
                    self.status_message = format!(
                        "Loaded {}: - {} files, {} directories",
                        drive, files, dirs
                    );

                    // Check if all drives are done
                    if self.trees.len() >= self.selected_drives.len() {
                        self.is_scanning = false;
                        self.scan_progress.clear();
                        let total_files: u64 =
                            self.trees.iter().map(|t| t.stats.total_files).sum();
                        let total_dirs: u64 =
                            self.trees.iter().map(|t| t.stats.total_directories).sum();
                        self.status_message =
                            format!("{} files, {} folders", total_files, total_dirs);
                        self.search.needs_search = true;
                        self.start_path_cache();
                    }
                }
                BgMessage::ScanError(msg) => {
                    self.status_message = msg;
                    // If this was the last drive (including errors), mark scanning done
                    if self.trees.len() >= self.selected_drives.len().saturating_sub(1) {
                        self.is_scanning = false;
                        self.scan_progress.clear();
                        if !self.all_entries.is_empty() {
                            self.search.needs_search = true;
                        }
                    }
                }
                BgMessage::SortComplete(column, sorted_indices) => {
                    self.filtered_indices = sorted_indices;
                    self.last_sort_column = Some(column);
                    self.last_sort_order = self.table.sort_order;
                    self.is_sorting = false;
                    self.status_message = format!("{} objects", self.filtered_indices.len());
                }
                BgMessage::MetadataRefreshComplete(updates) => {
                    for (entry_idx, file_size, modification_time) in updates {
                        if let Some(entry) = self.all_entries.get_mut(entry_idx) {
                            entry.file_size = file_size;
                            entry.modification_time = modification_time;
                        }
                        self.pending_metadata_refresh.remove(&entry_idx);
                    }
                    self.is_refreshing_metadata = false;
                }
                BgMessage::PathCacheComplete(paths) => {
                    for (entry_idx, path) in paths {
                        if let Some(entry) = self.all_entries.get_mut(entry_idx) {
                            entry.path_lower = path.to_lowercase();
                            entry.cached_path = path;
                        }
                    }
                    self.status_message = format!(
                        "{} objects (paths cached)",
                        self.filtered_indices.len()
                    );
                }
            }
        }
    }

    fn perform_search(&mut self) {
        self.filtered_indices.clear();
        self.last_sort_column = None;

        if self.trees.is_empty() || self.all_entries.is_empty() {
            return;
        }

        let raw_query = self.search.query.trim().to_lowercase();

        // Parse backtick-scoped path: `C:\path\to\folder` pattern
        let (scope_path, search_query) = parse_scope_path(&raw_query);

        // Build regex filter from search_filters if present
        let regex_filter = if !self.search_filters.regex_pattern.is_empty() {
            regex::Regex::new(&self.search_filters.regex_pattern).ok()
        } else {
            None
        };

        // Parse date filter
        let date_filter = self.build_date_filter();

        // Parse size filter
        let size_filter = self.build_size_filter();

        // Parse extension filter
        let ext_filter: Vec<String> = if !self.search_filters.extension_filter.is_empty() {
            self.search_filters
                .extension_filter
                .split(';')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        };

        let has_filters = regex_filter.is_some()
            || date_filter.is_some()
            || size_filter.is_some()
            || !ext_filter.is_empty();

        let no_text_query = search_query.is_empty();

        // Build pattern list from the text query
        let patterns: Vec<&str> = if no_text_query {
            Vec::new()
        } else {
            search_query
                .split(';')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect()
        };

        for (idx, entry) in self.all_entries.iter().enumerate() {
            // Scope path filter
            if let Some(ref scope) = scope_path {
                if entry.path_lower.is_empty()
                    || !entry.path_lower.starts_with(scope.as_str())
                {
                    continue;
                }
            }

            // Text pattern filter
            let text_match = if no_text_query {
                true
            } else {
                patterns.iter().any(|p| matches_pattern(&entry.name_lower, p))
            };

            if !text_match {
                continue;
            }

            // Regex filter
            if let Some(ref re) = regex_filter {
                if !re.is_match(&entry.name) {
                    continue;
                }
            }

            // Date filter
            if let Some(ref df) = date_filter {
                if !df.matches(entry.modification_time) {
                    continue;
                }
            }

            // Size filter
            if let Some(ref sf) = size_filter {
                if !sf.matches(entry.file_size) {
                    continue;
                }
            }

            // Extension filter
            if !ext_filter.is_empty() {
                if !ext_filter.contains(&entry.extension) {
                    continue;
                }
            }

            self.filtered_indices.push(idx);
        }

        // If no query and no filters, show everything
        if no_text_query && !has_filters && scope_path.is_none() {
            self.filtered_indices = (0..self.all_entries.len()).collect();
        }

        // Reset selection
        self.table.selected = if self.filtered_indices.is_empty() {
            None
        } else {
            Some(0)
        };
        self.table.scroll_offset = 0;
        self.table.selections.clear();
        if let Some(sel) = self.table.selected {
            self.table.selections.insert(sel);
            self.table.anchor = Some(sel);
        }

        self.trigger_metadata_refresh();
    }

    fn build_date_filter(&self) -> Option<DateFilter> {
        use crate::tui::menu::{parse_date_to_filetime, DateFilterMode};

        match self.search_filters.date_mode {
            DateFilterMode::None => None,
            DateFilterMode::After => {
                let start = parse_date_to_filetime(&self.search_filters.date_start)?;
                Some(DateFilter::After(start))
            }
            DateFilterMode::Before => {
                let end = parse_date_to_filetime(&self.search_filters.date_start)?;
                Some(DateFilter::Before(end))
            }
            DateFilterMode::Between => {
                let start = parse_date_to_filetime(&self.search_filters.date_start)?;
                let end = parse_date_to_filetime(&self.search_filters.date_end)?;
                Some(DateFilter::Between(start, end))
            }
        }
    }

    fn build_size_filter(&self) -> Option<SizeFilter> {
        use crate::tui::menu::{parse_size_str, SizeFilterMode};

        match self.search_filters.size_mode {
            SizeFilterMode::None => None,
            SizeFilterMode::GreaterThan => {
                let val = parse_size_str(&self.search_filters.size_value)?;
                Some(SizeFilter::GreaterThan(val))
            }
            SizeFilterMode::LessThan => {
                let val = parse_size_str(&self.search_filters.size_value)?;
                Some(SizeFilter::LessThan(val))
            }
            SizeFilterMode::Between => {
                let start = parse_size_str(&self.search_filters.size_value)?;
                let end = parse_size_str(&self.search_filters.size_end)?;
                Some(SizeFilter::Between(start, end))
            }
        }
    }

    fn trigger_metadata_refresh(&mut self) {
        if self.is_refreshing_metadata || self.filtered_indices.is_empty() {
            return;
        }

        let max_refresh = 10000;
        let mut needs_refresh: Vec<(usize, usize, NodeKey, u64)> = Vec::new();

        for &entry_idx in self.filtered_indices.iter().take(max_refresh) {
            if self.pending_metadata_refresh.contains(&entry_idx) {
                continue;
            }
            if let Some(entry) = self.all_entries.get(entry_idx) {
                if !entry.is_directory && (entry.file_size == 0 || entry.modification_time == 0) {
                    needs_refresh.push((
                        entry_idx,
                        entry.tree_index,
                        entry.key,
                        entry.file_reference_number,
                    ));
                    self.pending_metadata_refresh.insert(entry_idx);
                }
            }
        }

        if needs_refresh.is_empty() {
            return;
        }

        self.is_refreshing_metadata = true;

        let trees = self.trees.clone();
        let tx = match &self.bg_sender {
            Some(tx) => tx.clone(),
            None => return,
        };

        thread::spawn(move || {
            use std::collections::HashMap;

            let mut by_tree: HashMap<usize, Vec<(usize, NodeKey, u64)>> = HashMap::new();
            for (entry_idx, tree_idx, key, file_ref) in needs_refresh {
                by_tree
                    .entry(tree_idx)
                    .or_default()
                    .push((entry_idx, key, file_ref));
            }

            let mut updates: Vec<(usize, u64, u64)> = Vec::new();

            for (tree_idx, entries) in by_tree {
                if let Some(tree) = trees.get(tree_idx) {
                    let refresh_pairs: Vec<(NodeKey, u64)> = entries
                        .iter()
                        .map(|(_, key, fr)| (*key, *fr))
                        .collect();

                    let metadata_results = tree.refresh_metadata(&refresh_pairs);

                    for (entry_idx, key, _) in entries {
                        if let Some(&(file_size, modification_time)) = metadata_results.get(&key) {
                            updates.push((entry_idx, file_size, modification_time));
                        }
                    }
                }
            }

            let _ = tx.send(BgMessage::MetadataRefreshComplete(updates));
        });
    }

    fn start_path_cache(&self) {
        if self.all_entries.is_empty() {
            return;
        }

        let tx = match &self.bg_sender {
            Some(tx) => tx.clone(),
            None => return,
        };

        // Build a list of (index, tree_index, key) for all entries
        let work: Vec<(usize, usize, NodeKey)> = self
            .all_entries
            .iter()
            .enumerate()
            .map(|(i, e)| (i, e.tree_index, e.key))
            .collect();

        let trees = self.trees.clone();

        thread::spawn(move || {
            let mut results: Vec<(usize, String)> = Vec::with_capacity(work.len());

            for (idx, tree_index, key) in work {
                if let Some(tree) = trees.get(tree_index) {
                    let full_path = tree.build_path_for_key(&key);
                    let parent_dir = std::path::Path::new(&full_path)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| full_path);
                    results.push((idx, parent_dir));
                }
            }

            let _ = tx.send(BgMessage::PathCacheComplete(results));
        });
    }

    pub fn get_row_data(&self, entry_index: usize) -> Option<RowData> {
        let entry = self.all_entries.get(entry_index)?;
        let tree = self.trees.get(entry.tree_index)?;
        let _node = tree.get_by_key(&entry.key)?;

        let parent_dir = if !entry.cached_path.is_empty() {
            entry.cached_path.clone()
        } else {
            let path = tree.build_path_for_key(&entry.key);
            std::path::Path::new(&path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone())
        };

        Some(RowData {
            name: entry.name.clone(),
            path: parent_dir,
            file_size: entry.file_size,
            is_directory: entry.is_directory,
            modification_time: entry.modification_time,
            extension: entry.extension.clone(),
        })
    }

    pub fn handle_sort_click(&mut self, column: SortColumn) {
        if self.is_sorting {
            return;
        }

        let new_order = if self.table.sort_column == column {
            if self.table.sort_order == SortOrder::Ascending {
                SortOrder::Descending
            } else {
                SortOrder::Ascending
            }
        } else {
            SortOrder::Ascending
        };

        // Reverse optimization
        if self.last_sort_column == Some(column) && self.last_sort_order != new_order {
            self.filtered_indices.reverse();
            self.table.sort_column = column;
            self.table.sort_order = new_order;
            self.last_sort_order = new_order;
            return;
        }

        self.table.sort_column = column;
        self.table.sort_order = new_order;
        self.is_sorting = true;

        let mut indices = self.filtered_indices.clone();
        let entries = self.all_entries.clone();
        let sort_column = column;
        let sort_order = new_order;
        let trees = self.trees.clone();

        if let Some(tx) = &self.bg_sender {
            let tx = tx.clone();
            thread::spawn(move || {
                // For path sorting, build a full-path cache if needed
                let path_cache: Option<std::collections::HashMap<usize, String>> =
                    if sort_column == SortColumn::Path {
                        let mut cache = std::collections::HashMap::new();
                        for &idx in &indices {
                            let entry = &entries[idx];
                            let full_path = if !entry.path_lower.is_empty() {
                                // Combine cached parent path + filename for full path sort
                                format!("{}\\{}", entry.path_lower, entry.name_lower)
                            } else if let Some(tree) = trees.get(entry.tree_index) {
                                tree.build_path_for_key(&entry.key).to_lowercase()
                            } else {
                                entry.name_lower.clone()
                            };
                            cache.insert(idx, full_path);
                        }
                        Some(cache)
                    } else {
                        None
                    };

                indices.sort_by(|&a, &b| {
                    let ea = &entries[a];
                    let eb = &entries[b];

                    let cmp = match sort_column {
                        SortColumn::Name => ea.name_lower.cmp(&eb.name_lower),
                        SortColumn::Path => {
                            let pa = path_cache.as_ref().unwrap().get(&a).map(|s| s.as_str()).unwrap_or("");
                            let pb = path_cache.as_ref().unwrap().get(&b).map(|s| s.as_str()).unwrap_or("");
                            pa.cmp(pb)
                        }
                        SortColumn::Size => ea.file_size.cmp(&eb.file_size),
                        SortColumn::Extension => ea.extension.cmp(&eb.extension),
                        SortColumn::DateModified => {
                            ea.modification_time.cmp(&eb.modification_time)
                        }
                        SortColumn::Type => {
                            let ta = colors::type_label(ea.is_directory, &ea.extension);
                            let tb = colors::type_label(eb.is_directory, &eb.extension);
                            ta.cmp(tb)
                        }
                    };

                    if sort_order == SortOrder::Descending {
                        cmp.reverse()
                    } else {
                        cmp
                    }
                });

                let _ = tx.send(BgMessage::SortComplete(sort_column, indices));
            });
        }
    }

    // --- Key handling ---

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Global keys
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
                return;
            }
            _ => {}
        }

        // Route to active menu first
        if !matches!(self.active_menu, ActiveMenu::None) {
            self.handle_menu_key(key);
            return;
        }

        match key.code {
            KeyCode::Esc => {
                if self.search.focused && !self.search.query.is_empty() {
                    self.search.query.clear();
                    self.search.cursor_pos = 0;
                    self.search.needs_search = true;
                } else if self.search.focused {
                    self.search.focused = false;
                } else {
                    self.should_quit = true;
                }
                return;
            }
            KeyCode::F(9) => {
                self.start_scan();
                return;
            }
            _ => {}
        }

        if self.search.focused {
            self.handle_search_key(key);
        } else {
            self.handle_table_key(key);
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c) => {
                self.search.query.insert(self.search.cursor_pos, c);
                self.search.cursor_pos += c.len_utf8();
                self.search.needs_search = true;
            }
            KeyCode::Backspace => {
                if self.search.cursor_pos > 0 {
                    // Find the previous character boundary
                    let prev = self.search.query[..self.search.cursor_pos]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.search.query.remove(prev);
                    self.search.cursor_pos = prev;
                    self.search.needs_search = true;
                }
            }
            KeyCode::Delete => {
                if self.search.cursor_pos < self.search.query.len() {
                    self.search.query.remove(self.search.cursor_pos);
                    self.search.needs_search = true;
                }
            }
            KeyCode::Left => {
                if self.search.cursor_pos > 0 {
                    let prev = self.search.query[..self.search.cursor_pos]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.search.cursor_pos = prev;
                }
            }
            KeyCode::Right => {
                if self.search.cursor_pos < self.search.query.len() {
                    let next = self.search.query[self.search.cursor_pos..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.search.cursor_pos + i)
                        .unwrap_or(self.search.query.len());
                    self.search.cursor_pos = next;
                }
            }
            KeyCode::Home => {
                self.search.cursor_pos = 0;
            }
            KeyCode::End => {
                self.search.cursor_pos = self.search.query.len();
            }
            KeyCode::Tab | KeyCode::Down | KeyCode::Enter => {
                self.search.focused = false;
            }
            _ => {}
        }
    }

    fn handle_table_key(&mut self, key: KeyEvent) {
        let total = self.filtered_indices.len();
        let has_shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let has_ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            // Shift+Up/Down: extend selection
            KeyCode::Up if has_shift => self.table.shift_select_prev(),
            KeyCode::Down if has_shift => self.table.shift_select_next(total),

            // Ctrl+Up/Down: move cursor without changing selection
            KeyCode::Up if has_ctrl => self.table.move_prev(),
            KeyCode::Down if has_ctrl => self.table.move_next(total),

            // Normal Up/Down: single selection
            KeyCode::Up | KeyCode::Char('k') => self.table.select_prev(),
            KeyCode::Down | KeyCode::Char('j') => self.table.select_next(total),
            KeyCode::PageUp => self.table.page_up(),
            KeyCode::PageDown => self.table.page_down(total),
            KeyCode::Home => self.table.select_first(),
            KeyCode::End => self.table.select_last(total),

            // Space: toggle selection of current item
            KeyCode::Char(' ') => self.table.toggle_selection(),

            // Ctrl+A: select all
            KeyCode::Char('a') if has_ctrl => self.table.select_all(total),

            // Horizontal scroll
            KeyCode::Left if !has_ctrl => {
                self.table.horizontal_offset = self.table.horizontal_offset.saturating_sub(4);
            }
            KeyCode::Right if !has_ctrl => {
                self.table.horizontal_offset = self.table.horizontal_offset.saturating_add(4);
            }

            // Column resize (Ctrl+Left/Right resizes the current sort column)
            KeyCode::Left if has_ctrl => {
                let idx = self.table.sort_column.index();
                if self.table.column_widths[idx] != 0 {
                    self.table.column_widths[idx] =
                        self.table.column_widths[idx].saturating_sub(1).max(5);
                }
            }
            KeyCode::Right if has_ctrl => {
                let idx = self.table.sort_column.index();
                if self.table.column_widths[idx] != 0 {
                    self.table.column_widths[idx] =
                        (self.table.column_widths[idx] + 1).min(100);
                }
            }

            KeyCode::Tab | KeyCode::Char('/') => {
                self.search.focused = true;
            }

            // Sort columns
            KeyCode::F(1) => self.handle_sort_click(SortColumn::Name),
            KeyCode::F(2) => self.handle_sort_click(SortColumn::Path),
            KeyCode::F(3) => self.handle_sort_click(SortColumn::Size),
            KeyCode::F(4) => self.handle_sort_click(SortColumn::Extension),
            KeyCode::F(5) => self.handle_sort_click(SortColumn::DateModified),
            KeyCode::F(6) => self.handle_sort_click(SortColumn::Type),

            // Actions menu
            KeyCode::Char('m') if !has_ctrl && !has_shift => {
                self.open_actions_menu();
            }

            // Search filters menu
            KeyCode::Char('f') if has_ctrl => {
                self.open_search_filters();
            }

            // Any other printable char focuses search and types it
            KeyCode::Char(c) if !has_ctrl && !has_shift => {
                self.search.focused = true;
                self.search.query.push(c);
                self.search.cursor_pos = self.search.query.len();
                self.search.needs_search = true;
            }

            _ => {}
        }
    }

    // --- Menu methods ---

    fn open_actions_menu(&mut self) {
        if self.table.selected.is_some() {
            self.active_menu = ActiveMenu::Actions(ActionsMenu::new());
        }
    }

    fn open_search_filters(&mut self) {
        // Copy current persistent filters into a new menu
        let mut menu = SearchFiltersMenu::new();
        menu.regex_pattern = self.search_filters.regex_pattern.clone();
        menu.regex_cursor = self.search_filters.regex_pattern.len();
        menu.date_mode = self.search_filters.date_mode;
        menu.date_start = self.search_filters.date_start.clone();
        menu.date_start_cursor = self.search_filters.date_start.len();
        menu.date_end = self.search_filters.date_end.clone();
        menu.date_end_cursor = self.search_filters.date_end.len();
        menu.size_mode = self.search_filters.size_mode;
        menu.size_value = self.search_filters.size_value.clone();
        menu.size_value_cursor = self.search_filters.size_value.len();
        menu.size_end = self.search_filters.size_end.clone();
        menu.size_end_cursor = self.search_filters.size_end.len();
        menu.extension_filter = self.search_filters.extension_filter.clone();
        menu.extension_cursor = self.search_filters.extension_filter.len();
        self.active_menu = ActiveMenu::SearchFilters(menu);
    }

    /// Get the full paths for all selected items
    pub fn get_selected_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        for &logical_idx in &self.table.selections {
            if let Some(&entry_idx) = self.filtered_indices.get(logical_idx) {
                if let Some(entry) = self.all_entries.get(entry_idx) {
                    if !entry.cached_path.is_empty() {
                        let full = format!("{}\\{}", entry.cached_path, entry.name);
                        paths.push(full);
                    } else if let Some(tree) = self.trees.get(entry.tree_index) {
                        paths.push(tree.build_path_for_key(&entry.key));
                    }
                }
            }
        }
        // If no multi-selection but cursor is set, use cursor
        if paths.is_empty() {
            if let Some(logical_idx) = self.table.selected {
                if let Some(&entry_idx) = self.filtered_indices.get(logical_idx) {
                    if let Some(entry) = self.all_entries.get(entry_idx) {
                        if !entry.cached_path.is_empty() {
                            let full = format!("{}\\{}", entry.cached_path, entry.name);
                            paths.push(full);
                        } else if let Some(tree) = self.trees.get(entry.tree_index) {
                            paths.push(tree.build_path_for_key(&entry.key));
                        }
                    }
                }
            }
        }
        paths
    }

    fn execute_action(&mut self, action: ActionKind) {
        let paths = self.get_selected_paths();
        if paths.is_empty() {
            self.status_message = "No items selected".to_string();
            return;
        }

        match action {
            ActionKind::Open => {
                for path in &paths {
                    crate::tui::menu::open_file(path);
                }
                self.status_message = format!("Opened {} item(s)", paths.len());
            }
            ActionKind::OpenInExplorer => {
                for path in &paths {
                    crate::tui::menu::open_in_explorer(path);
                }
                self.status_message = format!("Opened in Explorer: {} item(s)", paths.len());
            }
            ActionKind::CopyPath => {
                let text = paths.join("\n");
                crate::tui::menu::copy_to_clipboard(&text);
                self.status_message = format!("Copied {} path(s) to clipboard", paths.len());
            }
            ActionKind::Delete => {
                let msg = if paths.len() == 1 {
                    format!("Delete {}?", paths[0])
                } else {
                    format!("Delete {} items?", paths.len())
                };
                self.active_menu =
                    ActiveMenu::Confirm(ConfirmDialog::new(msg, ActionKind::Delete));
                return;
            }
            ActionKind::Rename => {
                if paths.len() != 1 {
                    self.status_message = "Rename works on a single item only".to_string();
                    return;
                }
                let full_path = &paths[0];
                let name = std::path::Path::new(full_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                self.active_menu =
                    ActiveMenu::Rename(RenameDialog::new(name, full_path.clone()));
                return;
            }
        }
        self.active_menu = ActiveMenu::None;
    }

    fn execute_delete(&mut self) {
        let paths = self.get_selected_paths();
        let mut deleted = 0;
        for path in &paths {
            let p = std::path::Path::new(path);
            let result = if p.is_dir() {
                std::fs::remove_dir_all(p)
            } else {
                std::fs::remove_file(p)
            };
            match result {
                Ok(_) => deleted += 1,
                Err(e) => {
                    self.status_message = format!("Error deleting {}: {}", path, e);
                }
            }
        }
        if deleted > 0 {
            self.status_message = format!("Deleted {} item(s)", deleted);
            // Refresh the search to remove deleted items
            self.search.needs_search = true;
        }
    }

    fn execute_rename(&mut self, original_path: &str, new_name: &str) {
        let old = std::path::Path::new(original_path);
        let new = old.parent().map(|p| p.join(new_name));
        if let Some(new_path) = new {
            match std::fs::rename(old, &new_path) {
                Ok(_) => {
                    self.status_message =
                        format!("Renamed to {}", new_path.display());
                    self.search.needs_search = true;
                }
                Err(e) => {
                    self.status_message = format!("Rename error: {}", e);
                }
            }
        }
    }

    fn apply_search_filters(&mut self) {
        // Copy the menu's filter state to the persistent filters
        if let ActiveMenu::SearchFilters(ref menu) = self.active_menu {
            self.search_filters.regex_pattern = menu.regex_pattern.clone();
            self.search_filters.regex_cursor = menu.regex_pattern.len();
            self.search_filters.date_mode = menu.date_mode;
            self.search_filters.date_start = menu.date_start.clone();
            self.search_filters.date_start_cursor = menu.date_start.len();
            self.search_filters.date_end = menu.date_end.clone();
            self.search_filters.date_end_cursor = menu.date_end.len();
            self.search_filters.size_mode = menu.size_mode;
            self.search_filters.size_value = menu.size_value.clone();
            self.search_filters.size_value_cursor = menu.size_value.len();
            self.search_filters.size_end = menu.size_end.clone();
            self.search_filters.size_end_cursor = menu.size_end.len();
            self.search_filters.extension_filter = menu.extension_filter.clone();
            self.search_filters.extension_cursor = menu.extension_filter.len();
        }
        self.active_menu = ActiveMenu::None;
        self.search.needs_search = true;
    }

    fn handle_menu_key(&mut self, key: KeyEvent) {
        // Take ownership of the menu temporarily
        let menu = std::mem::replace(&mut self.active_menu, ActiveMenu::None);

        match menu {
            ActiveMenu::Actions(mut actions) => {
                match key.code {
                    KeyCode::Esc => {
                        // Already set to None above
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        actions.move_up();
                        self.active_menu = ActiveMenu::Actions(actions);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        actions.move_down();
                        self.active_menu = ActiveMenu::Actions(actions);
                    }
                    KeyCode::Enter => {
                        let action = actions.selected_action();
                        self.execute_action(action);
                    }
                    _ => {
                        self.active_menu = ActiveMenu::Actions(actions);
                    }
                }
            }
            ActiveMenu::Confirm(mut confirm) => {
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                        confirm.confirm_selected = !confirm.confirm_selected;
                        self.active_menu = ActiveMenu::Confirm(confirm);
                    }
                    KeyCode::Enter => {
                        if confirm.confirm_selected {
                            match confirm.action {
                                ActionKind::Delete => self.execute_delete(),
                                _ => {}
                            }
                        }
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        match confirm.action {
                            ActionKind::Delete => self.execute_delete(),
                            _ => {}
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') => {
                        // Cancel - already set to None
                    }
                    _ => {
                        self.active_menu = ActiveMenu::Confirm(confirm);
                    }
                }
            }
            ActiveMenu::Rename(mut rename) => {
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Enter => {
                        let path = rename.full_path.clone();
                        let new_name = rename.new_name.clone();
                        self.execute_rename(&path, &new_name);
                    }
                    KeyCode::Char(c) => {
                        rename.new_name.insert(rename.cursor_pos, c);
                        rename.cursor_pos += c.len_utf8();
                        self.active_menu = ActiveMenu::Rename(rename);
                    }
                    KeyCode::Backspace => {
                        if rename.cursor_pos > 0 {
                            let prev = rename.new_name[..rename.cursor_pos]
                                .char_indices()
                                .last()
                                .map(|(i, _)| i)
                                .unwrap_or(0);
                            rename.new_name.remove(prev);
                            rename.cursor_pos = prev;
                        }
                        self.active_menu = ActiveMenu::Rename(rename);
                    }
                    KeyCode::Delete => {
                        if rename.cursor_pos < rename.new_name.len() {
                            rename.new_name.remove(rename.cursor_pos);
                        }
                        self.active_menu = ActiveMenu::Rename(rename);
                    }
                    KeyCode::Left => {
                        if rename.cursor_pos > 0 {
                            let prev = rename.new_name[..rename.cursor_pos]
                                .char_indices()
                                .last()
                                .map(|(i, _)| i)
                                .unwrap_or(0);
                            rename.cursor_pos = prev;
                        }
                        self.active_menu = ActiveMenu::Rename(rename);
                    }
                    KeyCode::Right => {
                        if rename.cursor_pos < rename.new_name.len() {
                            let next = rename.new_name[rename.cursor_pos..]
                                .char_indices()
                                .nth(1)
                                .map(|(i, _)| rename.cursor_pos + i)
                                .unwrap_or(rename.new_name.len());
                            rename.cursor_pos = next;
                        }
                        self.active_menu = ActiveMenu::Rename(rename);
                    }
                    KeyCode::Home => {
                        rename.cursor_pos = 0;
                        self.active_menu = ActiveMenu::Rename(rename);
                    }
                    KeyCode::End => {
                        rename.cursor_pos = rename.new_name.len();
                        self.active_menu = ActiveMenu::Rename(rename);
                    }
                    _ => {
                        self.active_menu = ActiveMenu::Rename(rename);
                    }
                }
            }
            ActiveMenu::SearchFilters(mut filters) => {
                match key.code {
                    KeyCode::Esc => {
                        // Cancel - already set to None
                    }
                    KeyCode::Tab => {
                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            filters.focused_field = filters.focused_field.prev();
                        } else {
                            filters.focused_field = filters.focused_field.next();
                        }
                        self.active_menu = ActiveMenu::SearchFilters(filters);
                    }
                    KeyCode::Enter => {
                        match filters.focused_field {
                            SearchFilterField::Apply => {
                                self.active_menu = ActiveMenu::SearchFilters(filters);
                                self.apply_search_filters();
                                return;
                            }
                            SearchFilterField::Clear => {
                                filters.clear_all();
                                self.active_menu = ActiveMenu::SearchFilters(filters);
                            }
                            SearchFilterField::Cancel => {
                                // Cancel - already set to None
                            }
                            _ => {
                                // Enter on a text field = Apply
                                self.active_menu = ActiveMenu::SearchFilters(filters);
                                self.apply_search_filters();
                                return;
                            }
                        }
                    }
                    KeyCode::Left if filters.focused_field.is_mode_selector() => {
                        match filters.focused_field {
                            SearchFilterField::DateMode => {
                                filters.date_mode = filters.date_mode.prev();
                            }
                            SearchFilterField::SizeMode => {
                                filters.size_mode = filters.size_mode.prev();
                            }
                            _ => {}
                        }
                        self.active_menu = ActiveMenu::SearchFilters(filters);
                    }
                    KeyCode::Right if filters.focused_field.is_mode_selector() => {
                        match filters.focused_field {
                            SearchFilterField::DateMode => {
                                filters.date_mode = filters.date_mode.next();
                            }
                            SearchFilterField::SizeMode => {
                                filters.size_mode = filters.size_mode.next();
                            }
                            _ => {}
                        }
                        self.active_menu = ActiveMenu::SearchFilters(filters);
                    }
                    KeyCode::Char(c) if filters.focused_field.is_text_input() => {
                        if let Some((text, cursor)) = filters.current_text_mut() {
                            text.insert(*cursor, c);
                            *cursor += c.len_utf8();
                        }
                        self.active_menu = ActiveMenu::SearchFilters(filters);
                    }
                    KeyCode::Backspace if filters.focused_field.is_text_input() => {
                        if let Some((text, cursor)) = filters.current_text_mut() {
                            if *cursor > 0 {
                                let prev = text[..*cursor]
                                    .char_indices()
                                    .last()
                                    .map(|(i, _)| i)
                                    .unwrap_or(0);
                                text.remove(prev);
                                *cursor = prev;
                            }
                        }
                        self.active_menu = ActiveMenu::SearchFilters(filters);
                    }
                    KeyCode::Delete if filters.focused_field.is_text_input() => {
                        if let Some((text, cursor)) = filters.current_text_mut() {
                            if *cursor < text.len() {
                                text.remove(*cursor);
                            }
                        }
                        self.active_menu = ActiveMenu::SearchFilters(filters);
                    }
                    KeyCode::Left if filters.focused_field.is_text_input() => {
                        if let Some((text, cursor)) = filters.current_text_mut() {
                            if *cursor > 0 {
                                let prev = text[..*cursor]
                                    .char_indices()
                                    .last()
                                    .map(|(i, _)| i)
                                    .unwrap_or(0);
                                *cursor = prev;
                            }
                        }
                        self.active_menu = ActiveMenu::SearchFilters(filters);
                    }
                    KeyCode::Right if filters.focused_field.is_text_input() => {
                        if let Some((text, cursor)) = filters.current_text_mut() {
                            if *cursor < text.len() {
                                let next = text[*cursor..]
                                    .char_indices()
                                    .nth(1)
                                    .map(|(i, _)| *cursor + i)
                                    .unwrap_or(text.len());
                                *cursor = next;
                            }
                        }
                        self.active_menu = ActiveMenu::SearchFilters(filters);
                    }
                    _ => {
                        self.active_menu = ActiveMenu::SearchFilters(filters);
                    }
                }
            }
            ActiveMenu::None => unreachable!(),
        }
    }
}

fn extract_extension(name: &str) -> String {
    if let Some(dot_pos) = name.rfind('.') {
        if dot_pos > 0 && dot_pos < name.len() - 1 {
            return name[dot_pos + 1..].to_lowercase();
        }
    }
    String::new()
}

/// Date filter for search
enum DateFilter {
    After(u64),
    Before(u64),
    Between(u64, u64),
}

impl DateFilter {
    fn matches(&self, modification_time: u64) -> bool {
        if modification_time == 0 {
            return false;
        }
        match self {
            DateFilter::After(start) => modification_time >= *start,
            DateFilter::Before(end) => modification_time <= *end,
            DateFilter::Between(start, end) => {
                modification_time >= *start && modification_time <= *end
            }
        }
    }
}

/// Size filter for search
enum SizeFilter {
    GreaterThan(u64),
    LessThan(u64),
    Between(u64, u64),
}

impl SizeFilter {
    fn matches(&self, file_size: u64) -> bool {
        match self {
            SizeFilter::GreaterThan(val) => file_size > *val,
            SizeFilter::LessThan(val) => file_size < *val,
            SizeFilter::Between(start, end) => file_size >= *start && file_size <= *end,
        }
    }
}

/// Parse a search query for backtick-scoped path.
/// E.g., `` `C:\Users\jdoe` *.docx `` returns (Some("c:\\users\\jdoe"), "*.docx")
fn parse_scope_path(query: &str) -> (Option<String>, String) {
    if let Some(start) = query.find('`') {
        if let Some(end) = query[start + 1..].find('`') {
            let scope = query[start + 1..start + 1 + end].trim().to_string();
            let rest_before = query[..start].trim();
            let rest_after = query[start + 1 + end + 1..].trim();
            let remaining = format!("{} {}", rest_before, rest_after)
                .trim()
                .to_string();
            if !scope.is_empty() {
                return (Some(scope), remaining);
            }
        }
    }
    (None, query.to_string())
}
