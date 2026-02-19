use crate::file_tree::NodeKey;
use crate::gui::colors;
use crate::gui::dialogs::{self, SearchFilters};
use crate::gui::search::{matches_pattern, SearchState};
use crate::gui::table::{SortColumn, SortOrder, TableState};
use crate::gui::treemap::TreemapState;
use crate::{FileTree, MultiVolumeScanner, ScanConfig, VolumeScanner};
use eframe::egui;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;

// ============================================================================
// Background messages
// ============================================================================

pub enum BgMessage {
    ScanProgress(String),
    ScanComplete(Arc<FileTree>),
    ScanError(String),
    SortComplete(SortColumn, Vec<usize>),
    MetadataRefreshComplete(Vec<(usize, u64, u64)>),
    PathCacheComplete(Vec<(usize, String)>),
}

// ============================================================================
// Cached entry data (same as TUI)
// ============================================================================

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

// ============================================================================
// Preset filter
// ============================================================================

#[derive(Debug, Clone)]
pub struct PresetFilter {
    pub name: String,
    pub search: String,
    pub macro_name: String,
}

// ============================================================================
// Dialog state
// ============================================================================

enum ActiveDialog {
    None,
    SearchFilters(SearchFilters),
    Confirm { message: String, action: PendingAction },
    Rename { original_path: String, original_name: String, new_name: String },
    Info { title: String, lines: Vec<String> },
}

#[derive(Clone)]
enum PendingAction {
    Delete,
}

#[derive(Clone)]
struct ContextMenu {
    path: String,
    name: String,
    pos: egui::Pos2,
}

// ============================================================================
// Main GUI App
// ============================================================================

pub struct GuiApp {
    // Data
    trees: Vec<Arc<FileTree>>,
    all_entries: Vec<EntryData>,
    filtered_indices: Vec<usize>,

    // Sub-states
    search: SearchState,
    table: TableState,

    // Scanning
    is_scanning: bool,
    is_sorting: bool,
    is_refreshing_metadata: bool,
    scan_progress: String,
    status_message: String,
    total_count: u64,

    // Drives
    selected_drives: Vec<char>,

    // Sort cache
    last_sort_column: Option<SortColumn>,
    last_sort_order: SortOrder,

    // Channel
    bg_receiver: Option<Receiver<BgMessage>>,
    bg_sender: Option<Sender<BgMessage>>,

    // Metadata
    pending_metadata_refresh: std::collections::HashSet<usize>,

    // Dialogs
    active_dialog: ActiveDialog,

    // Persistent search filters
    search_filters: SearchFilters,

    // Treemap
    treemap: Option<TreemapState>,

    // Context menu
    context_menu: Option<ContextMenu>,

    // Preset filters
    preset_filters: Vec<PresetFilter>,

    // Search focus flag (for auto-focus TextEdit)
    request_search_focus: bool,
}

impl GuiApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let available_drives = MultiVolumeScanner::detect_ntfs_volumes();
        let selected_drives = available_drives.clone();
        let preset_filters = load_preset_filters();

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
            active_dialog: ActiveDialog::None,
            search_filters: SearchFilters::new(),
            treemap: None,
            preset_filters,
            request_search_focus: true,
            context_menu: None,
        };

        if !app.selected_drives.is_empty() {
            app.start_scan();
        }

        app
    }

    // ====================================================================
    // Scanning
    // ====================================================================

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
                    show_progress: false,
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

    // ====================================================================
    // Message processing (called every frame)
    // ====================================================================

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

                    if self.trees.iter().any(|t| t.drive_letter == drive) {
                        continue;
                    }

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
                    self.status_message =
                        format!("Loaded {}: - {} files, {} directories", drive, files, dirs);

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

    // ====================================================================
    // Search
    // ====================================================================

    fn perform_search(&mut self) {
        self.filtered_indices.clear();
        self.last_sort_column = None;

        if self.trees.is_empty() || self.all_entries.is_empty() {
            return;
        }

        let raw_query = self.search.query.trim().to_lowercase();
        let (scope_path, search_query) = parse_scope_path(&raw_query);

        let regex_filter = if !self.search_filters.regex_pattern.is_empty() {
            regex::Regex::new(&self.search_filters.regex_pattern).ok()
        } else {
            None
        };

        let date_filter = self.build_date_filter();
        let size_filter = self.build_size_filter();

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
            if let Some(ref scope) = scope_path {
                if entry.path_lower.is_empty()
                    || !entry.path_lower.starts_with(scope.as_str())
                {
                    continue;
                }
            }

            let text_match = if no_text_query {
                true
            } else {
                patterns
                    .iter()
                    .any(|p| matches_pattern(&entry.name_lower, p))
            };

            if !text_match {
                continue;
            }

            if let Some(ref re) = regex_filter {
                if !re.is_match(&entry.name) {
                    continue;
                }
            }

            if let Some(ref df) = date_filter {
                if !df.matches(entry.modification_time) {
                    continue;
                }
            }

            if let Some(ref sf) = size_filter {
                if !sf.matches(entry.file_size) {
                    continue;
                }
            }

            if !ext_filter.is_empty() && !ext_filter.contains(&entry.extension) {
                continue;
            }

            self.filtered_indices.push(idx);
        }

        if no_text_query && !has_filters && scope_path.is_none() {
            self.filtered_indices = (0..self.all_entries.len()).collect();
        }

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
        match self.search_filters.date_mode {
            dialogs::DateFilterMode::None => None,
            dialogs::DateFilterMode::After => {
                let start = dialogs::parse_date_to_filetime(&self.search_filters.date_start)?;
                Some(DateFilter::After(start))
            }
            dialogs::DateFilterMode::Before => {
                let end = dialogs::parse_date_to_filetime(&self.search_filters.date_start)?;
                Some(DateFilter::Before(end))
            }
            dialogs::DateFilterMode::Between => {
                let start = dialogs::parse_date_to_filetime(&self.search_filters.date_start)?;
                let end = dialogs::parse_date_to_filetime(&self.search_filters.date_end)?;
                Some(DateFilter::Between(start, end))
            }
        }
    }

    fn build_size_filter(&self) -> Option<SizeFilter> {
        match self.search_filters.size_mode {
            dialogs::SizeFilterMode::None => None,
            dialogs::SizeFilterMode::GreaterThan => {
                let val = dialogs::parse_size_str(&self.search_filters.size_value)?;
                Some(SizeFilter::GreaterThan(val))
            }
            dialogs::SizeFilterMode::LessThan => {
                let val = dialogs::parse_size_str(&self.search_filters.size_value)?;
                Some(SizeFilter::LessThan(val))
            }
            dialogs::SizeFilterMode::Between => {
                let start = dialogs::parse_size_str(&self.search_filters.size_value)?;
                let end = dialogs::parse_size_str(&self.search_filters.size_end)?;
                Some(SizeFilter::Between(start, end))
            }
        }
    }

    // ====================================================================
    // Metadata & path cache
    // ====================================================================

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

    // ====================================================================
    // Sort
    // ====================================================================

    fn handle_sort_click(&mut self, column: SortColumn) {
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
                let path_cache: Option<std::collections::HashMap<usize, String>> =
                    if sort_column == SortColumn::Path {
                        let mut cache = std::collections::HashMap::new();
                        for &idx in &indices {
                            let entry = &entries[idx];
                            let full_path = if !entry.path_lower.is_empty() {
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
                            let pa = path_cache
                                .as_ref()
                                .unwrap()
                                .get(&a)
                                .map(|s| s.as_str())
                                .unwrap_or("");
                            let pb = path_cache
                                .as_ref()
                                .unwrap()
                                .get(&b)
                                .map(|s| s.as_str())
                                .unwrap_or("");
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

    // ====================================================================
    // Actions
    // ====================================================================

    fn get_selected_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        for &logical_idx in &self.table.selections {
            if let Some(&entry_idx) = self.filtered_indices.get(logical_idx) {
                if let Some(entry) = self.all_entries.get(entry_idx) {
                    if !entry.cached_path.is_empty() {
                        paths.push(format!("{}\\{}", entry.cached_path, entry.name));
                    } else if let Some(tree) = self.trees.get(entry.tree_index) {
                        paths.push(tree.build_path_for_key(&entry.key));
                    }
                }
            }
        }
        if paths.is_empty() {
            if let Some(logical_idx) = self.table.selected {
                if let Some(&entry_idx) = self.filtered_indices.get(logical_idx) {
                    if let Some(entry) = self.all_entries.get(entry_idx) {
                        if !entry.cached_path.is_empty() {
                            paths.push(format!("{}\\{}", entry.cached_path, entry.name));
                        } else if let Some(tree) = self.trees.get(entry.tree_index) {
                            paths.push(tree.build_path_for_key(&entry.key));
                        }
                    }
                }
            }
        }
        paths
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
            self.search.needs_search = true;
        }
    }

    fn execute_rename(&mut self, original_path: &str, new_name: &str) {
        let old = std::path::Path::new(original_path);
        let new = old.parent().map(|p| p.join(new_name));
        if let Some(new_path) = new {
            match std::fs::rename(old, &new_path) {
                Ok(_) => {
                    self.status_message = format!("Renamed to {}", new_path.display());
                    self.search.needs_search = true;
                }
                Err(e) => {
                    self.status_message = format!("Rename error: {}", e);
                }
            }
        }
    }

    fn apply_preset_filter(&mut self, filter: &PresetFilter) {
        let search = &filter.search;

        if search.is_empty() {
            self.search_filters.clear_all();
            self.search.query.clear();
            self.search.needs_search = true;
            self.status_message = format!("Filter: {}", filter.name);
            return;
        }

        if search == "folder:" {
            self.search_filters.clear_all();
            self.search.query.clear();
            self.search.needs_search = false;

            self.filtered_indices.clear();
            self.last_sort_column = None;
            for (idx, entry) in self.all_entries.iter().enumerate() {
                if entry.is_directory {
                    self.filtered_indices.push(idx);
                }
            }
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
            self.status_message = format!("Filter: {}", filter.name);
            return;
        }

        if let Some(ext_list) = search.strip_prefix("ext:") {
            self.search_filters.clear_all();
            self.search_filters.extension_filter = ext_list.to_string();
            self.search.query.clear();
            self.search.needs_search = true;
            self.status_message = format!("Filter: {}", filter.name);
        }
    }

    // ====================================================================
    // Row data builder (for visible rows only)
    // ====================================================================

    fn get_parent_path(&self, entry_idx: usize) -> String {
        let entry = &self.all_entries[entry_idx];
        if !entry.cached_path.is_empty() {
            entry.cached_path.clone()
        } else if let Some(tree) = self.trees.get(entry.tree_index) {
            let path = tree.build_path_for_key(&entry.key);
            std::path::Path::new(&path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or(path)
        } else {
            String::new()
        }
    }
}

// ============================================================================
// eframe::App implementation — the main render loop
// ============================================================================

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Always repaint while scanning/sorting so the UI stays live
        if self.is_scanning || self.is_sorting || self.is_refreshing_metadata {
            ctx.request_repaint();
        }

        // Process background messages
        self.process_messages();

        // Deferred search
        if self.search.needs_search && !self.is_scanning {
            self.perform_search();
            self.search.needs_search = false;
        }

        // ── Dark theme ──────────────────────────────────────────────────
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(egui::Color32::from_rgb(220, 220, 220));
        ctx.set_visuals(visuals);

        // ── Keyboard shortcuts ──────────────────────────────────────────
        ctx.input(|i| {
            if i.key_pressed(egui::Key::F9) {
                self.start_scan();
            }
        });

        // ── Treemap fullscreen view ─────────────────────────────────────
        if self.treemap.is_some() {
            self.draw_treemap_view(ctx);
            return;
        }

        // ── Dialogs ─────────────────────────────────────────────────────
        self.handle_dialogs(ctx);

        // ── Menu bar ────────────────────────────────────────────────────
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                // File menu
                ui.menu_button("File", |ui| {
                    if ui.button("Open").clicked() {
                        let paths = self.get_selected_paths();
                        for p in &paths {
                            dialogs::open_file(p);
                        }
                        ui.close();
                    }
                    if ui.button("Open in Explorer").clicked() {
                        let paths = self.get_selected_paths();
                        for p in &paths {
                            dialogs::open_in_explorer(p);
                        }
                        ui.close();
                    }
                    if ui.button("Properties").clicked() {
                        let paths = self.get_selected_paths();
                        for p in &paths {
                            dialogs::show_properties(p);
                        }
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Rescan  (F9)").clicked() {
                        self.start_scan();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Quit  (Ctrl+Q)").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                // Edit menu
                ui.menu_button("Edit", |ui| {
                    if ui.button("Copy Path").clicked() {
                        let paths = self.get_selected_paths();
                        dialogs::copy_to_clipboard(&paths.join("\n"));
                        self.status_message = format!("Copied {} path(s)", paths.len());
                        ui.close();
                    }
                    if ui.button("Select All  (Ctrl+A)").clicked() {
                        let total = self.filtered_indices.len();
                        self.table.select_all(total);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Rename").clicked() {
                        let paths = self.get_selected_paths();
                        if paths.len() == 1 {
                            let name = std::path::Path::new(&paths[0])
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            self.active_dialog = ActiveDialog::Rename {
                                original_path: paths[0].clone(),
                                original_name: name.clone(),
                                new_name: name,
                            };
                        }
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        let paths = self.get_selected_paths();
                        if !paths.is_empty() {
                            let msg = if paths.len() == 1 {
                                format!("Delete {}?", paths[0])
                            } else {
                                format!("Delete {} items?", paths.len())
                            };
                            self.active_dialog = ActiveDialog::Confirm {
                                message: msg,
                                action: PendingAction::Delete,
                            };
                        }
                        ui.close();
                    }
                });

                // View menu
                ui.menu_button("View", |ui| {
                    if ui.button("Treemap  (T)").clicked() {
                        self.toggle_treemap();
                        ui.close();
                    }
                    if ui.button("Search Filters  (Ctrl+F)").clicked() {
                        self.active_dialog =
                            ActiveDialog::SearchFilters(self.search_filters.clone());
                        ui.close();
                    }
                });

                // Tools menu
                ui.menu_button("Tools", |ui| {
                    let filters: Vec<PresetFilter> = self.preset_filters.clone();
                    for filter in &filters {
                        if ui.button(format!("Filter: {}", filter.name)).clicked() {
                            self.apply_preset_filter(filter);
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("Clear All Filters").clicked() {
                        self.search_filters.clear_all();
                        self.search.query.clear();
                        self.search.needs_search = true;
                        self.status_message = "Filters cleared".to_string();
                        ui.close();
                    }
                });

                // Help menu
                ui.menu_button("Help", |ui| {
                    if ui.button("Keyboard Shortcuts").clicked() {
                        self.active_dialog = ActiveDialog::Info {
                            title: "Keyboard Shortcuts".to_string(),
                            lines: vec![
                                "Ctrl+F          Search filters".into(),
                                "Ctrl+A          Select all".into(),
                                "F9              Rescan drives".into(),
                                "T               Toggle treemap".into(),
                                "Enter           Open file".into(),
                                "Delete          Delete file(s)".into(),
                                "F2              Rename file".into(),
                                "Ctrl+C          Copy path".into(),
                                "Ctrl+Q          Quit".into(),
                                "".into(),
                                "Click column headers to sort.".into(),
                            ],
                        };
                        ui.close();
                    }
                    if ui.button("About EmFit").clicked() {
                        self.active_dialog = ActiveDialog::Info {
                            title: "About EmFit".to_string(),
                            lines: vec![
                                format!("EmFit v{}", crate::VERSION),
                                "".into(),
                                "Ultra-fast NTFS file scanner".into(),
                                "".into(),
                                "Combines direct MFT reading with".into(),
                                "USN Journal for instant file".into(),
                                "enumeration and accurate sizes.".into(),
                            ],
                        };
                        ui.close();
                    }
                });
            });
        });

        // ── Status bar (bottom) ─────────────────────────────────────────
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(24.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if self.is_scanning {
                        ui.spinner();
                        ui.label(
                            egui::RichText::new(format!("Scanning... {}", self.scan_progress))
                                .color(egui::Color32::from_rgb(80, 200, 255)),
                        );
                    } else if self.is_sorting {
                        ui.spinner();
                        ui.label("Sorting...");
                    } else {
                        let obj_count = self.filtered_indices.len();
                        let selected_count = self
                            .table
                            .selections
                            .len()
                            .max(if self.table.selected.is_some() { 1 } else { 0 });
                        let total_size: u64 =
                            self.trees.iter().map(|t| t.stats.total_size).sum();
                        ui.label(format!(
                            "{} objects | {} selected | {} total",
                            obj_count,
                            selected_count,
                            crate::format_size(total_size)
                        ));
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(&self.status_message)
                                .color(egui::Color32::from_rgb(160, 160, 160)),
                        );
                    });
                });
            });

        // ── Central panel: search + table ───────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            // Search bar
            ui.horizontal(|ui| {
                ui.label("\u{1F50D}");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search.query)
                        .desired_width(ui.available_width() - 120.0)
                        .hint_text("Search files... (use ; to separate patterns, `path` to scope)")
                        .font(egui::TextStyle::Body),
                );
                if self.request_search_focus {
                    response.request_focus();
                    self.request_search_focus = false;
                }
                if response.changed() {
                    self.search.needs_search = true;
                }

                // Filters button
                let filter_label = if self.search_filters.has_any_filter() {
                    "\u{1F50D} Filters \u{2713}"
                } else {
                    "\u{1F50D} Filters"
                };
                if ui.button(filter_label).clicked() {
                    self.active_dialog = ActiveDialog::SearchFilters(self.search_filters.clone());
                }

                // Treemap button
                if ui.button("\u{1F4CA} Treemap").clicked() {
                    self.toggle_treemap();
                }
            });

            ui.separator();

            // Table
            self.draw_file_table(ui);
        });

        // ── Global keyboard shortcuts ───────────────────────────────────
        ctx.input(|i| {
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Q) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::F) {
                // Can't borrow self mutably here, so just flag it
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::A) {
                // handled below
            }
        });
        // Deferred ctrl+F and ctrl+A (needs &mut self)
        let ctrl_f = ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::F));
        let ctrl_a = ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::A));
        if ctrl_f {
            self.active_dialog = ActiveDialog::SearchFilters(self.search_filters.clone());
        }
        if ctrl_a {
            let total = self.filtered_indices.len();
            self.table.select_all(total);
        }
    }
}

// ============================================================================
// Drawing helpers
// ============================================================================

impl GuiApp {
    /// Draw the main file table with sortable headers.
    fn draw_file_table(&mut self, ui: &mut egui::Ui) {
        use egui_extras::{Column, TableBuilder};

        let total = self.filtered_indices.len();
        let row_height = 20.0;

        // Collect sort click outside the borrow
        let mut sort_click: Option<SortColumn> = None;
        // Collect row actions
        let mut open_path: Option<String> = None;

        // Cache sort state to avoid borrowing self in closures
        let current_sort_col = self.table.sort_column;
        let current_sort_order = self.table.sort_order;
        let ctx = ui.ctx().clone();

        let table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(220.0).at_least(60.0)) // Name
            .column(Column::remainder().at_least(100.0))   // Path
            .column(Column::initial(100.0).at_least(50.0)) // Size
            .column(Column::initial(70.0).at_least(40.0))  // Ext
            .column(Column::initial(150.0).at_least(80.0)) // Date Modified
            .column(Column::initial(120.0).at_least(50.0)) // Type
            .sense(egui::Sense::click())
            .min_scrolled_height(0.0);

        let columns: [(& str, SortColumn); 6] = [
            ("Name", SortColumn::Name),
            ("Path", SortColumn::Path),
            ("Size", SortColumn::Size),
            ("Ext", SortColumn::Extension),
            ("Date Modified", SortColumn::DateModified),
            ("Type", SortColumn::Type),
        ];

        table
            .header(22.0, |mut header| {
                for (label, col) in &columns {
                    header.col(|ui| {
                        let text = if current_sort_col == *col {
                            format!("{}{}", label, current_sort_order.indicator())
                        } else {
                            label.to_string()
                        };
                        let response = ui.add(
                            egui::Label::new(
                                egui::RichText::new(text)
                                    .strong()
                                    .color(egui::Color32::WHITE),
                            )
                            .sense(egui::Sense::click()),
                        );
                        if response.clicked() {
                            sort_click = Some(*col);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(row_height, total, |mut row| {
                    let logical_idx = row.index();
                    let is_selected = self.table.selections.contains(&logical_idx)
                        || self.table.selected == Some(logical_idx);

                    // Highlight selected rows
                    row.set_selected(is_selected);

                    if let Some(&entry_idx) = self.filtered_indices.get(logical_idx) {
                        let entry = &self.all_entries[entry_idx];
                        let icon = colors::icon_for_entry(entry.is_directory, &entry.extension);
                        let name_color = if entry.is_directory {
                            egui::Color32::from_rgb(100, 180, 255)
                        } else {
                            colors::color_for_extension(&entry.extension)
                        };

                        let parent_path = self.get_parent_path(entry_idx);

                        let size_str = if entry.is_directory {
                            String::new()
                        } else {
                            crate::format_size(entry.file_size)
                        };
                        let date_str = if entry.modification_time > 0 {
                            crate::format_filetime(entry.modification_time)
                        } else {
                            String::new()
                        };
                        let type_str = colors::type_label(entry.is_directory, &entry.extension);

                        // Name
                        row.col(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{} {}", icon, entry.name))
                                    .color(name_color),
                            );
                        });
                        // Path
                        row.col(|ui| {
                            ui.label(
                                egui::RichText::new(&parent_path)
                                    .color(egui::Color32::from_rgb(160, 160, 160)),
                            );
                        });
                        // Size
                        row.col(|ui| {
                            ui.label(
                                egui::RichText::new(&size_str)
                                    .color(egui::Color32::from_rgb(80, 200, 80)),
                            );
                        });
                        // Ext
                        row.col(|ui| {
                            ui.label(
                                egui::RichText::new(&entry.extension)
                                    .color(egui::Color32::from_rgb(100, 140, 255)),
                            );
                        });
                        // Date
                        row.col(|ui| {
                            ui.label(&date_str);
                        });
                        // Type
                        row.col(|ui| {
                            ui.label(
                                egui::RichText::new(type_str)
                                    .color(egui::Color32::from_rgb(140, 140, 140))
                                    .italics(),
                            );
                        });

                        // Click handling
                        let response = row.response();
                        if response.clicked() {
                            self.table.selected = Some(logical_idx);
                            self.table.selections.clear();
                            self.table.selections.insert(logical_idx);
                            self.table.anchor = Some(logical_idx);
                        }
                        if response.double_clicked() {
                            let full_path = if !entry.cached_path.is_empty() {
                                format!("{}\\{}", entry.cached_path, entry.name)
                            } else if let Some(tree) = self.trees.get(entry.tree_index) {
                                tree.build_path_for_key(&entry.key)
                            } else {
                                String::new()
                            };
                            if !full_path.is_empty() {
                                open_path = Some(full_path);
                            }
                        }
                        if response.secondary_clicked() {
                            self.table.selected = Some(logical_idx);
                            self.table.selections.clear();
                            self.table.selections.insert(logical_idx);
                            let full_path = if !entry.cached_path.is_empty() {
                                format!("{}\\{}", entry.cached_path, entry.name)
                            } else if let Some(tree) = self.trees.get(entry.tree_index) {
                                tree.build_path_for_key(&entry.key)
                            } else {
                                String::new()
                            };
                            let pos = ctx.pointer_latest_pos().unwrap_or_default();
                            self.context_menu = Some(ContextMenu {
                                path: full_path,
                                name: entry.name.clone(),
                                pos,
                            });
                        }
                    }
                });
            });

        // Handle sort
        if let Some(col) = sort_click {
            self.handle_sort_click(col);
        }

        // Handle double-click open
        if let Some(path) = open_path {
            dialogs::open_file(&path);
        }

        // Context menu popup
        self.draw_context_menu(ui);
    }

    fn draw_context_menu(&mut self, ui: &mut egui::Ui) {
        if let Some(cm) = self.context_menu.clone() {
            let area_resp = egui::Area::new(ui.id().with("ctx_menu"))
                .fixed_pos(cm.pos)
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(160.0);
                        if ui.button("Open").clicked() {
                            dialogs::open_file(&cm.path);
                            self.context_menu = None;
                        }
                        if ui.button("Open in Explorer").clicked() {
                            dialogs::open_in_explorer(&cm.path);
                            self.context_menu = None;
                        }
                        if ui.button("Properties").clicked() {
                            dialogs::show_properties(&cm.path);
                            self.context_menu = None;
                        }
                        ui.separator();
                        if ui.button("Copy Path").clicked() {
                            dialogs::copy_to_clipboard(&cm.path);
                            self.status_message = "Path copied".to_string();
                            self.context_menu = None;
                        }
                        ui.separator();
                        if ui.button("Rename").clicked() {
                            self.active_dialog = ActiveDialog::Rename {
                                original_path: cm.path.clone(),
                                original_name: cm.name.clone(),
                                new_name: cm.name.clone(),
                            };
                            self.context_menu = None;
                        }
                        if ui.button("Delete").clicked() {
                            self.active_dialog = ActiveDialog::Confirm {
                                message: format!("Delete {}?", cm.path),
                                action: PendingAction::Delete,
                            };
                            self.context_menu = None;
                        }
                    });
                });

            // Close context menu when clicking outside
            if ui.ctx().input(|i| i.pointer.any_pressed()) {
                if !area_resp.response.contains_pointer() {
                    self.context_menu = None;
                }
            }
        }
    }

    /// Handle active dialog rendering.
    fn handle_dialogs(&mut self, ctx: &egui::Context) {
        let dialog = std::mem::replace(&mut self.active_dialog, ActiveDialog::None);

        match dialog {
            ActiveDialog::None => {}
            ActiveDialog::SearchFilters(mut filters) => {
                let mut applied = false;
                let still_open =
                    dialogs::show_search_filters_dialog(ctx, &mut filters, &mut applied);
                if applied {
                    self.search_filters = filters;
                    self.search.needs_search = true;
                } else if still_open {
                    self.active_dialog = ActiveDialog::SearchFilters(filters);
                }
            }
            ActiveDialog::Confirm { message, action } => {
                if let Some(result) = dialogs::show_confirm_dialog(ctx, &message) {
                    if result {
                        match action {
                            PendingAction::Delete => self.execute_delete(),
                        }
                    }
                } else {
                    self.active_dialog = ActiveDialog::Confirm { message, action };
                }
            }
            ActiveDialog::Rename {
                original_path,
                original_name,
                mut new_name,
            } => {
                let result = dialogs::show_rename_dialog(ctx, &mut new_name, &original_name);
                match result {
                    Some(Some(name)) => {
                        self.execute_rename(&original_path, &name);
                    }
                    Some(None) => {
                        // cancelled
                    }
                    None => {
                        self.active_dialog = ActiveDialog::Rename {
                            original_path,
                            original_name,
                            new_name,
                        };
                    }
                }
            }
            ActiveDialog::Info { title, lines } => {
                if dialogs::show_info_dialog(ctx, &title, &lines) {
                    self.active_dialog = ActiveDialog::Info { title, lines };
                }
            }
        }
    }

    // ====================================================================
    // Treemap
    // ====================================================================

    fn toggle_treemap(&mut self) {
        if self.treemap.is_some() {
            self.treemap = None;
        } else {
            let mut state = TreemapState::new();
            state.build_from_trees(&self.trees);
            self.treemap = Some(state);
        }
    }

    fn draw_treemap_view(&mut self, ctx: &egui::Context) {
        // Top panel: breadcrumb + back / close buttons
        egui::TopBottomPanel::top("treemap_toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("\u{2190} Back").clicked() {
                    self.treemap_go_up();
                }
                if ui.button("\u{2716} Close").clicked() {
                    self.treemap = None;
                    return;
                }

                ui.separator();

                // Breadcrumb
                if let Some(ref tm) = self.treemap {
                    let crumb: String = tm
                        .breadcrumb
                        .iter()
                        .map(|(_, n)| n.as_str())
                        .collect::<Vec<_>>()
                        .join(" \u{25B8} ");
                    ui.label(
                        egui::RichText::new(format!("\u{1F4C1} {}", crumb))
                            .color(egui::Color32::WHITE),
                    );
                }
            });
        });

        // Bottom panel: info bar
        egui::TopBottomPanel::bottom("treemap_info")
            .exact_height(24.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(ref tm) = self.treemap {
                        if let Some(r) = tm.selected_rect() {
                            let icon = if r.is_directory {
                                "\u{1F4C1}"
                            } else {
                                "\u{1F4C4}"
                            };
                            ui.label(format!(
                                "{} {} — {}",
                                icon,
                                r.name,
                                crate::format_size(r.size)
                            ));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label("Click to select · Double-click to drill down · Backspace/Back to go up");
                    });
                });
            });

        // Central: the treemap canvas
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(15, 15, 25)))
            .show(ctx, |ui| {
                let available = ui.available_size();
                let (response, painter) = ui.allocate_painter(
                    available,
                    egui::Sense::click(),
                );

                let canvas_rect = response.rect;

                // Update canvas size in treemap state (rebuild if changed significantly)
                let needs_rebuild = {
                    let tm = self.treemap.as_ref().unwrap();
                    (canvas_rect.width() as f64 - tm.canvas_w).abs() > 2.0
                        || (canvas_rect.height() as f64 - tm.canvas_h).abs() > 2.0
                };
                if needs_rebuild {
                    let current_key = self.treemap.as_ref().unwrap().current_key;
                    let breadcrumb = self.treemap.as_ref().unwrap().breadcrumb.clone();
                    let tm = self.treemap.as_mut().unwrap();
                    tm.set_canvas_size(canvas_rect.width(), canvas_rect.height());
                    if current_key == NodeKey::root() {
                        tm.build_from_trees(&self.trees);
                    } else if let Some(tree) =
                        self.trees.iter().find(|t| t.get_by_key(&current_key).is_some())
                    {
                        let tree_clone = tree.clone();
                        tm.build_from_node(&tree_clone, &current_key);
                        tm.breadcrumb = breadcrumb;
                    }
                }

                let tm = self.treemap.as_ref().unwrap();
                let cw = canvas_rect.width() as f64;
                let ch = canvas_rect.height() as f64;

                // Draw rects
                let mut clicked_idx: Option<usize> = None;
                let mut double_clicked_idx: Option<usize> = None;

                for (i, rect) in tm.rects.iter().enumerate() {
                    let rx = canvas_rect.left() + (rect.x * cw) as f32;
                    let ry = canvas_rect.top() + (rect.y * ch) as f32;
                    let rw = ((rect.x + rect.w) * cw) as f32 - (rect.x * cw) as f32;
                    let rh = ((rect.y + rect.h) * ch) as f32 - (rect.y * ch) as f32;

                    if rw < 1.0 || rh < 1.0 {
                        continue;
                    }

                    let r = egui::Rect::from_min_size(
                        egui::pos2(rx, ry),
                        egui::vec2(rw, rh),
                    );

                    let is_sel = i == tm.selected;

                    if rect.children_rendered {
                        // Container: border + dark bg
                        let bg = colors::depth_bg_color(rect.depth);
                        let border = colors::depth_border_color(rect.depth);
                        painter.rect_filled(r, 0.0, bg);
                        let stroke_color = if is_sel {
                            egui::Color32::from_rgb(255, 255, 100)
                        } else {
                            border
                        };
                        painter.rect_stroke(
                            r,
                            0.0,
                            egui::Stroke::new(if is_sel { 2.0 } else { 1.0 }, stroke_color),
                            egui::StrokeKind::Outside,
                        );
                        // Title
                        if rw > 30.0 {
                            let title = fit_title(&rect.name, rect.size, rw as usize);
                            if !title.is_empty() {
                                painter.text(
                                    egui::pos2(rx + 4.0, ry + 2.0),
                                    egui::Align2::LEFT_TOP,
                                    &title,
                                    egui::FontId::proportional(11.0),
                                    egui::Color32::WHITE,
                                );
                            }
                        }
                    } else {
                        // Leaf
                        let bg = if is_sel {
                            egui::Color32::from_rgb(0, 0, 170) // CGA blue
                        } else {
                            colors::leaf_color(&rect.name, rect.is_directory, i)
                        };
                        painter.rect_filled(r, 0.0, bg);
                        painter.rect_stroke(
                            r,
                            0.0,
                            egui::Stroke::new(0.5, egui::Color32::from_rgba_premultiplied(0, 0, 0, 80)),
                            egui::StrokeKind::Outside,
                        );

                        // Name + size text
                        if rw > 20.0 && rh > 12.0 {
                            let name_trunc = trunc(&rect.name, (rw / 7.0) as usize);
                            painter.text(
                                egui::pos2(rx + 2.0, ry + 2.0),
                                egui::Align2::LEFT_TOP,
                                &name_trunc,
                                egui::FontId::proportional(11.0),
                                egui::Color32::from_rgb(235, 235, 235),
                            );
                            if rh > 26.0 {
                                painter.text(
                                    egui::pos2(rx + 2.0, ry + 14.0),
                                    egui::Align2::LEFT_TOP,
                                    crate::format_size(rect.size),
                                    egui::FontId::proportional(10.0),
                                    egui::Color32::from_rgb(200, 200, 200),
                                );
                            }
                        }
                    }

                    // Hit test
                    if let Some(pos) = response.interact_pointer_pos() {
                        if r.contains(pos) && !rect.children_rendered {
                            clicked_idx = Some(i);
                        }
                    }
                    // Double click detection via response
                    if response.double_clicked() {
                        if let Some(pos) = ui.ctx().pointer_latest_pos() {
                            if r.contains(pos) {
                                double_clicked_idx = Some(i);
                            }
                        }
                    }
                }

                // Handle click
                if let Some(idx) = clicked_idx {
                    if let Some(ref mut tm) = self.treemap {
                        tm.selected = idx;
                    }
                }

                // Handle double-click: drill down
                if let Some(idx) = double_clicked_idx {
                    self.treemap_drill_down(idx);
                }
            });

        // Keyboard: Backspace to go up
        let go_up = ctx.input(|i| i.key_pressed(egui::Key::Backspace));
        let press_t = ctx.input(|i| i.key_pressed(egui::Key::T));
        let press_escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let press_right = ctx.input(|i| i.key_pressed(egui::Key::ArrowRight));
        let press_left = ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft));
        let press_enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));

        if go_up {
            self.treemap_go_up();
        }
        if press_t || press_escape {
            self.treemap = None;
        }
        if press_right {
            if let Some(ref mut tm) = self.treemap {
                tm.move_next();
            }
        }
        if press_left {
            if let Some(ref mut tm) = self.treemap {
                tm.move_prev();
            }
        }
        if press_enter {
            let sel = self.treemap.as_ref().map(|tm| tm.selected).unwrap_or(0);
            self.treemap_drill_down(sel);
        }
    }

    fn treemap_drill_down(&mut self, idx: usize) {
        let drill_info = self.treemap.as_ref().and_then(|tm| {
            tm.rects.get(idx).and_then(|rect| {
                if rect.is_directory {
                    Some((rect.key, rect.name.clone()))
                } else {
                    None
                }
            })
        });

        if let Some((key, name)) = drill_info {
            let tree_ref = self
                .trees
                .iter()
                .find(|t| t.get_by_key(&key).is_some());
            if let Some(tree) = tree_ref {
                let tree_clone = tree.clone();
                if let Some(ref mut tm) = self.treemap {
                    tm.breadcrumb.push((key, name));
                    tm.build_from_node(&tree_clone, &key);
                }
            }
        }
    }

    fn treemap_go_up(&mut self) {
        let parent_info = self.treemap.as_mut().and_then(|tm| {
            if tm.breadcrumb.len() > 1 {
                tm.breadcrumb.pop();
                let (parent_key, _) = tm.breadcrumb.last().cloned()?;
                Some(parent_key)
            } else {
                None
            }
        });

        if let Some(parent_key) = parent_info {
            if parent_key == NodeKey::root() {
                if let Some(ref mut tm) = self.treemap {
                    tm.build_from_trees(&self.trees);
                }
            } else {
                let tree_ref = self
                    .trees
                    .iter()
                    .find(|t| t.get_by_key(&parent_key).is_some());
                if let Some(tree) = tree_ref {
                    let tree_clone = tree.clone();
                    if let Some(ref mut tm) = self.treemap {
                        tm.build_from_node(&tree_clone, &parent_key);
                    }
                }
            }
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

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

fn extract_extension(name: &str) -> String {
    if let Some(dot_pos) = name.rfind('.') {
        if dot_pos > 0 && dot_pos < name.len() - 1 {
            return name[dot_pos + 1..].to_lowercase();
        }
    }
    String::new()
}

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

fn fit_title(name: &str, size: u64, max_chars: usize) -> String {
    if max_chars < 6 {
        return String::new();
    }
    let size_str = crate::format_size(size);
    let full = format!("{} ({})", name, size_str);
    if full.len() <= max_chars {
        return full;
    }
    if name.len() <= max_chars {
        return name.to_string();
    }
    let trunc_len = max_chars.saturating_sub(1);
    if trunc_len == 0 {
        return String::new();
    }
    let t: String = name.chars().take(trunc_len).collect();
    format!("{}\u{2026}", t)
}

fn trunc(s: &str, max: usize) -> String {
    let cc = s.chars().count();
    if cc <= max {
        s.to_string()
    } else if max <= 1 {
        s.chars().take(max).collect()
    } else {
        let t: String = s.chars().take(max - 1).collect();
        format!("{}\u{2026}", t)
    }
}

/// Load preset filters from Filters.csv (same logic as TUI).
fn load_preset_filters() -> Vec<PresetFilter> {
    let mut filters = Vec::new();

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    let csv_path = if let Some(dir) = exe_dir {
        dir.join("Filters.csv")
    } else {
        std::path::PathBuf::from("Filters.csv")
    };

    if let Ok(content) = std::fs::read_to_string(&csv_path) {
        let mut lines = content.lines();
        let _header = lines.next();

        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            if let Some(filter) = parse_filter_csv_line(line) {
                filters.push(filter);
            }
        }
    }

    filters
}

fn parse_filter_csv_line(line: &str) -> Option<PresetFilter> {
    let fields = parse_csv_fields(line);
    if fields.len() < 7 {
        return None;
    }

    let name = fields[0].trim_matches('"').to_string();
    let search = fields[6].trim_matches('"').to_string();
    let macro_name = if fields.len() > 7 {
        fields[7].trim_matches('"').to_string()
    } else {
        String::new()
    };

    if name.is_empty() {
        return None;
    }

    Some(PresetFilter {
        name,
        search,
        macro_name,
    })
}

fn parse_csv_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in line.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
            current.push(ch);
        } else if ch == ',' && !in_quotes {
            fields.push(current.clone());
            current.clear();
        } else {
            current.push(ch);
        }
    }
    fields.push(current);

    fields
}
