use crate::file_tree::NodeKey;
use crate::tui::colors;
use crate::tui::search::{matches_pattern, SearchState};
use crate::tui::table::{SortColumn, SortOrder, TableState};
use crate::tui::ui;
use crate::{FileTree, MultiVolumeScanner, ScanConfig, VolumeScanner};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
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
                    self.handle_key(key);
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

        if raw_query.is_empty() {
            self.filtered_indices = (0..self.all_entries.len()).collect();
        } else {
            let patterns: Vec<&str> = raw_query
                .split(';')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();

            for (idx, entry) in self.all_entries.iter().enumerate() {
                for pattern in &patterns {
                    if matches_pattern(&entry.name_lower, pattern) {
                        self.filtered_indices.push(idx);
                        break;
                    }
                }
            }
        }

        // Reset selection
        self.table.selected = if self.filtered_indices.is_empty() {
            None
        } else {
            Some(0)
        };
        self.table.scroll_offset = 0;

        self.trigger_metadata_refresh();
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

    pub fn get_row_data(&self, entry_index: usize) -> Option<RowData> {
        let entry = self.all_entries.get(entry_index)?;
        let tree = self.trees.get(entry.tree_index)?;
        let _node = tree.get_by_key(&entry.key)?;
        let path = tree.build_path_for_key(&entry.key);
        let parent_dir = std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());

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

        if let Some(tx) = &self.bg_sender {
            let tx = tx.clone();
            thread::spawn(move || {
                indices.sort_by(|&a, &b| {
                    let ea = &entries[a];
                    let eb = &entries[b];

                    let cmp = match sort_column {
                        SortColumn::Name => ea.name_lower.cmp(&eb.name_lower),
                        SortColumn::Path => std::cmp::Ordering::Equal,
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
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.table.select_prev(),
            KeyCode::Down | KeyCode::Char('j') => self.table.select_next(total),
            KeyCode::PageUp => self.table.page_up(),
            KeyCode::PageDown => self.table.page_down(total),
            KeyCode::Home => self.table.select_first(),
            KeyCode::End => self.table.select_last(total),

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

            // Any other printable char focuses search and types it
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search.focused = true;
                self.search.query.push(c);
                self.search.cursor_pos = self.search.query.len();
                self.search.needs_search = true;
            }

            _ => {}
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
