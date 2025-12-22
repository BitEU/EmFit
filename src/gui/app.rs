//! Main EmFit Application

use crate::file_tree::NodeKey;
use crate::gui::search::SearchState;
use crate::gui::table::{ResultsTable, SortColumn, SortOrder};
use crate::{FileTree, MultiVolumeScanner, ScanConfig, TreeNode, VolumeScanner};
use eframe::egui;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;

/// Message types for background operations
pub enum BackgroundMessage {
    ScanProgress(String),
    ScanComplete(Arc<FileTree>),
    ScanError(String),
    SortComplete(SortColumn, Vec<usize>),
    SortProgress(String),
    MetadataRefreshComplete(Vec<(usize, u64, u64)>), // (entry_index, file_size, modification_time)
}

/// A lightweight entry with cached sort keys
#[derive(Clone)]
pub struct EntryData {
    pub tree_index: usize,
    /// Composite key (record_number, parent_record_number) for unique identification
    pub key: NodeKey,
    /// Full file reference number (for OpenFileById metadata refresh)
    pub file_reference_number: u64,
    pub name_lower: String,  // Pre-lowercased for fast sorting
    pub file_size: u64,
    pub modification_time: u64,
    pub is_directory: bool,
}

/// Main application state
pub struct EmFitApp {
    /// Search state
    search: SearchState,
    /// Results table
    table: ResultsTable,
    /// Loaded file trees per drive
    trees: Vec<Arc<FileTree>>,
    /// All entries with cached sort data
    all_entries: Vec<EntryData>,
    /// Filtered entry indices (into all_entries)
    filtered_indices: Vec<usize>,
    /// Currently scanning
    is_scanning: bool,
    /// Currently sorting
    is_sorting: bool,
    /// Scan progress message
    scan_progress: String,
    /// Channel for background messages
    bg_receiver: Option<Receiver<BackgroundMessage>>,
    /// Sender for sort operations
    sort_sender: Option<Sender<BackgroundMessage>>,
    /// Selected drives to scan
    selected_drives: Vec<char>,
    /// Available NTFS drives
    available_drives: Vec<char>,
    /// Show about dialog
    show_about: bool,
    /// Status bar message
    status_message: String,
    /// Total file count
    total_count: u64,
    /// Last sort column (for reverse optimization)
    last_sort_column: Option<SortColumn>,
    /// Last sort order (for reverse optimization)
    last_sort_order: SortOrder,
    /// Whether metadata refresh is in progress
    is_refreshing_metadata: bool,
    /// Set of entry indices that need metadata refresh (have 0 size or 0 time)
    pending_metadata_refresh: std::collections::HashSet<usize>,
}

impl Default for EmFitApp {
    fn default() -> Self {
        let available_drives = MultiVolumeScanner::detect_ntfs_volumes();
        let selected_drives = available_drives.clone();

        Self {
            search: SearchState::default(),
            table: ResultsTable::default(),
            trees: Vec::new(),
            all_entries: Vec::new(),
            filtered_indices: Vec::new(),
            is_scanning: false,
            is_sorting: false,
            scan_progress: String::new(),
            bg_receiver: None,
            sort_sender: None,
            selected_drives,
            available_drives,
            show_about: false,
            status_message: "Ready".to_string(),
            total_count: 0,
            last_sort_column: None,
            last_sort_order: SortOrder::Ascending,
            is_refreshing_metadata: false,
            pending_metadata_refresh: std::collections::HashSet::new(),
        }
    }
}

impl EmFitApp {
    /// Create a new EmFitApp
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::default();

        // Auto-start scanning if drives are available
        if !app.available_drives.is_empty() {
            app.start_scan();
        }

        app
    }

    /// Start scanning selected drives
    fn start_scan(&mut self) {
        if self.is_scanning || self.selected_drives.is_empty() {
            return;
        }

        self.is_scanning = true;
        self.scan_progress = "Starting scan...".to_string();
        self.trees.clear();
        self.all_entries.clear();
        self.filtered_indices.clear();
        self.table.clear();
        self.total_count = 0;
        self.last_sort_column = None;

        let (tx, rx) = channel();
        self.bg_receiver = Some(rx);
        self.sort_sender = Some(tx.clone());

        let drives = self.selected_drives.clone();

        thread::spawn(move || {
            for drive in drives {
                let _ = tx.send(BackgroundMessage::ScanProgress(format!(
                    "Scanning {}:...",
                    drive
                )));

                let config = ScanConfig {
                    use_usn: true,
                    use_mft: true,
                    include_hidden: true,
                    include_system: true,
                    calculate_sizes: true, // Must be true to get file sizes from MFT
                    show_progress: false,
                    batch_size: 1024,
                };

                let mut scanner = VolumeScanner::new(drive).with_config(config);

                match scanner.scan() {
                    Ok(tree) => {
                        let _ = tx.send(BackgroundMessage::ScanComplete(Arc::new(tree)));
                    }
                    Err(e) => {
                        let _ = tx.send(BackgroundMessage::ScanError(format!(
                            "Error scanning {}: {}",
                            drive, e
                        )));
                    }
                }
            }
        });
    }

    /// Process background messages
    fn process_messages(&mut self) {
        if let Some(rx) = &self.bg_receiver {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    BackgroundMessage::ScanProgress(msg) => {
                        self.scan_progress = msg;
                    }
                    BackgroundMessage::ScanComplete(tree) => {
                        let drive = tree.drive_letter;
                        let files = tree.stats.total_files;
                        let dirs = tree.stats.total_directories;

                        // Check if we already have a tree for this drive (prevent duplicates)
                        if self.trees.iter().any(|t| t.drive_letter == drive) {
                            // Skip duplicate scan completion
                            continue;
                        }

                        // Build entry data with cached sort keys
                        let tree_index = self.trees.len();
                        for entry in tree.iter() {
                            let key = *entry.key();
                            let node = entry.value();
                            if !node.name.is_empty() {
                                self.all_entries.push(EntryData {
                                    tree_index,
                                    key,
                                    file_reference_number: node.file_reference_number,
                                    name_lower: node.name.to_lowercase(),
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

                        // Check if we're done
                        if self.trees.len() >= self.selected_drives.len() {
                            self.is_scanning = false;
                            self.scan_progress.clear();
                            let total_files: u64 =
                                self.trees.iter().map(|t| t.stats.total_files).sum();
                            let total_dirs: u64 =
                                self.trees.iter().map(|t| t.stats.total_directories).sum();
                            self.status_message =
                                format!("{} files, {} folders", total_files, total_dirs);

                            // Trigger initial search to show all files
                            self.search.needs_search = true;
                        }
                    }
                    BackgroundMessage::ScanError(msg) => {
                        self.status_message = msg;
                        if self.trees.len() >= self.selected_drives.len().saturating_sub(1) {
                            self.is_scanning = false;
                            self.scan_progress.clear();
                        }
                    }
                    BackgroundMessage::SortProgress(msg) => {
                        self.status_message = msg;
                    }
                    BackgroundMessage::SortComplete(column, sorted_indices) => {
                        self.filtered_indices = sorted_indices;
                        self.last_sort_column = Some(column);
                        self.last_sort_order = self.table.sort_order;
                        self.is_sorting = false;
                        self.status_message = format!("{} objects", self.filtered_indices.len());
                    }
                    BackgroundMessage::MetadataRefreshComplete(updates) => {
                        // Apply metadata updates to cached entries
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
    }

    /// Render menu bar
    fn render_menu(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Rescan").clicked() {
                        self.start_scan();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Edit", |ui| {
                    if ui.button("Copy Path").clicked() {
                        if let Some(selected_idx) = self.table.selected {
                            if let Some(&entry_idx) = self.filtered_indices.get(selected_idx) {
                                if let Some(data) = self.get_row_data(entry_idx) {
                                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                        let _ = clipboard.set_text(&data.path);
                                    }
                                }
                            }
                        }
                        ui.close_menu();
                    }
                    if ui.button("Copy Name").clicked() {
                        if let Some(selected_idx) = self.table.selected {
                            if let Some(&entry_idx) = self.filtered_indices.get(selected_idx) {
                                if let Some(entry) = self.all_entries.get(entry_idx) {
                                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                        // Get original name from tree
                                        if let Some(tree) = self.trees.get(entry.tree_index) {
                                            if let Some(node) = tree.get_by_key(&entry.key) {
                                                let _ = clipboard.set_text(&node.name);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Select All").clicked() {
                        ui.close_menu();
                    }
                });

                ui.menu_button("Help", |ui| {
                    if ui.button("About EmFit").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    /// Render search bar
    fn render_search_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("search_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Search:");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search.query)
                        .desired_width(ui.available_width() - 10.0)
                        .hint_text("Type to search...")
                );

                if self.search.first_frame {
                    response.request_focus();
                    self.search.first_frame = false;
                }

                if response.changed() {
                    self.search.needs_search = true;
                }
            });
        });
    }

    /// Render status bar
    fn render_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.is_scanning {
                    ui.spinner();
                    ui.label(&self.scan_progress);
                } else if self.is_sorting {
                    ui.spinner();
                    ui.label("Sorting...");
                } else {
                    let result_count = self.filtered_indices.len();
                    ui.label(format!("{} objects", result_count));

                    ui.separator();

                    if let Some(selected_idx) = self.table.selected {
                        if let Some(&entry_idx) = self.filtered_indices.get(selected_idx) {
                            if let Some(entry) = self.all_entries.get(entry_idx) {
                                if let Some(tree) = self.trees.get(entry.tree_index) {
                                    let path = tree.build_path_for_key(&entry.key);
                                    ui.label(format!(
                                        "Size: {}, Path: {}",
                                        crate::format_size(entry.file_size),
                                        path
                                    ));
                                } else {
                                    ui.label(&self.status_message);
                                }
                            } else {
                                ui.label(&self.status_message);
                            }
                        } else {
                            ui.label(&self.status_message);
                        }
                    } else {
                        ui.label(&self.status_message);
                    }
                }
            });
        });
    }

    /// Render about dialog
    fn render_about_dialog(&mut self, ctx: &egui::Context) {
        if self.show_about {
            egui::Window::new("About EmFit")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.heading("EmFit");
                        ui.label(format!("Version {}", crate::VERSION));
                        ui.add_space(10.0);
                        ui.label("Ultra-fast NTFS file scanner");
                        ui.label("Combines USN Journal and MFT reading");
                        ui.add_space(10.0);
                        if ui.button("OK").clicked() {
                            self.show_about = false;
                        }
                    });
                });
        }
    }

    /// Perform search across all loaded trees
    fn perform_search(&mut self) {
        self.filtered_indices.clear();
        self.last_sort_column = None; // Reset sort state after search

        if self.trees.is_empty() || self.all_entries.is_empty() {
            return;
        }

        let query = self.search.query.trim().to_lowercase();

        if query.is_empty() {
            self.filtered_indices = (0..self.all_entries.len()).collect();
        } else {
            for (idx, entry) in self.all_entries.iter().enumerate() {
                if entry.name_lower.contains(&query) {
                    self.filtered_indices.push(idx);
                }
            }
        }

        // Trigger metadata refresh for results with missing data
        self.trigger_metadata_refresh();
    }

    /// Trigger background metadata refresh for entries with 0 size or 0 modification time
    fn trigger_metadata_refresh(&mut self) {
        if self.is_refreshing_metadata || self.filtered_indices.is_empty() {
            return;
        }

        // Collect entries that need metadata refresh (0 size or 0 time for non-directories)
        // Limit to visible results to avoid excessive API calls
        let max_refresh = 10000; // Reasonable limit
        // (entry_idx, tree_idx, key, file_reference_number)
        let mut needs_refresh: Vec<(usize, usize, NodeKey, u64)> = Vec::new();

        for &entry_idx in self.filtered_indices.iter().take(max_refresh) {
            if self.pending_metadata_refresh.contains(&entry_idx) {
                continue; // Already pending
            }

            if let Some(entry) = self.all_entries.get(entry_idx) {
                // Non-directories with 0 size or 0 modification time need refresh
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

        // Group by tree index for efficient batch processing
        let trees = self.trees.clone();
        let tx = match &self.sort_sender {
            Some(tx) => tx.clone(),
            None => return,
        };

        thread::spawn(move || {
            use std::collections::HashMap;

            // Group by tree index: tree_idx -> Vec<(entry_idx, key, file_ref)>
            let mut by_tree: HashMap<usize, Vec<(usize, NodeKey, u64)>> = HashMap::new();
            for (entry_idx, tree_idx, key, file_ref) in needs_refresh {
                by_tree.entry(tree_idx).or_default().push((entry_idx, key, file_ref));
            }

            let mut updates: Vec<(usize, u64, u64)> = Vec::new();

            // Process each tree
            for (tree_idx, entries) in by_tree {
                if let Some(tree) = trees.get(tree_idx) {
                    // Build pairs of (NodeKey, file_reference_number)
                    let refresh_pairs: Vec<(NodeKey, u64)> = entries
                        .iter()
                        .map(|(_, key, fr)| (*key, *fr))
                        .collect();

                    // Refresh metadata in the tree using full FRN
                    let metadata_results = tree.refresh_metadata(&refresh_pairs);

                    // Collect updated values
                    for (entry_idx, key, _) in entries {
                        if let Some(&(file_size, modification_time)) = metadata_results.get(&key) {
                            updates.push((entry_idx, file_size, modification_time));
                        }
                    }
                }
            }

            let _ = tx.send(BackgroundMessage::MetadataRefreshComplete(updates));
        });
    }

    /// Get a node by entry index
    pub fn get_node(&self, entry_index: usize) -> Option<(TreeNode, &FileTree)> {
        let entry = self.all_entries.get(entry_index)?;
        let tree = self.trees.get(entry.tree_index)?;
        let node = tree.get_by_key(&entry.key)?;
        Some((node, tree))
    }
}

impl eframe::App for EmFitApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.process_messages();

        if self.search.needs_search && !self.is_scanning {
            self.perform_search();
            self.search.needs_search = false;
        }

        self.render_menu(ctx, frame);
        self.render_search_bar(ctx);
        self.render_status_bar(ctx);
        self.render_about_dialog(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_results_table(ui);
        });

        if self.is_scanning || self.is_sorting || self.is_refreshing_metadata {
            ctx.request_repaint();
        }
    }
}

/// Row data extracted for rendering
struct RowData {
    name: String,
    path: String,
    file_size: u64,
    is_directory: bool,
    modification_time: u64,
}

impl EmFitApp {
    /// Get row data for a specific entry index (only builds path when needed for display)
    fn get_row_data(&self, entry_index: usize) -> Option<RowData> {
        let entry = self.all_entries.get(entry_index)?;
        let tree = self.trees.get(entry.tree_index)?;
        let node = tree.get_by_key(&entry.key)?;
        let path = tree.build_path_for_key(&entry.key);
        // Show the directory containing the item in the Path column (exclude the file/dir name)
        // Use Path::parent() for robust handling of root and drive paths.
        let parent_dir = std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());

        Some(RowData {
            name: node.name,
            path: parent_dir,
            file_size: entry.file_size,
            is_directory: entry.is_directory,
            modification_time: entry.modification_time,
        })
    }

    /// Render the results table with virtual scrolling
    fn render_results_table(&mut self, ui: &mut egui::Ui) {
        use egui_extras::{Column, TableBuilder};

        let available_height = ui.available_height();
        let row_count = self.filtered_indices.len();

        let name_header = if self.table.sort_column == SortColumn::Name {
            format!("Name{}", self.table.sort_order.indicator())
        } else {
            "Name".to_string()
        };
        let path_header = if self.table.sort_column == SortColumn::Path {
            format!("Path{}", self.table.sort_order.indicator())
        } else {
            "Path".to_string()
        };
        let size_header = if self.table.sort_column == SortColumn::Size {
            format!("Size{}", self.table.sort_order.indicator())
        } else {
            "Size".to_string()
        };
        let date_header = if self.table.sort_column == SortColumn::DateModified {
            format!("Date Modified{}", self.table.sort_order.indicator())
        } else {
            "Date Modified".to_string()
        };

        let mut clicked_column: Option<SortColumn> = None;
        let mut new_selection: Option<usize> = None;

        let sort_column = self.table.sort_column;
        let current_selection = self.table.selected;

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(200.0).at_least(20.0).clip(true))
            .column(Column::remainder().at_least(20.0).clip(true))
            .column(Column::initial(80.0).at_least(20.0).clip(true))
            .column(Column::initial(130.0).at_least(20.0).clip(true))
            .min_scrolled_height(0.0)
            .max_scroll_height(available_height)
            .sense(egui::Sense::click())
            .header(20.0, |mut header| {
                header.col(|ui| {
                    if ui.selectable_label(sort_column == SortColumn::Name, &name_header).clicked() {
                        clicked_column = Some(SortColumn::Name);
                    }
                });
                header.col(|ui| {
                    if ui.selectable_label(sort_column == SortColumn::Path, &path_header).clicked() {
                        clicked_column = Some(SortColumn::Path);
                    }
                });
                header.col(|ui| {
                    if ui.selectable_label(sort_column == SortColumn::Size, &size_header).clicked() {
                        clicked_column = Some(SortColumn::Size);
                    }
                });
                header.col(|ui| {
                    if ui.selectable_label(sort_column == SortColumn::DateModified, &date_header).clicked() {
                        clicked_column = Some(SortColumn::DateModified);
                    }
                });
            })
            .body(|body| {
                body.rows(18.0, row_count, |mut row| {
                    let row_index = row.index();
                    let is_selected = current_selection == Some(row_index);

                    if is_selected {
                        row.set_selected(true);
                    }

                    if let Some(&entry_idx) = self.filtered_indices.get(row_index) {
                        if let Some(data) = self.get_row_data(entry_idx) {
                            row.col(|ui| {
                                let icon = if data.is_directory { "\u{1F4C1}" } else { "\u{1F4C4}" };
                                let text = format!("{} {}", icon, data.name);
                                if ui.selectable_label(is_selected, &text).clicked() {
                                    new_selection = Some(row_index);
                                }
                            });
                            row.col(|ui| {
                                if ui.selectable_label(is_selected, &data.path).clicked() {
                                    new_selection = Some(row_index);
                                }
                            });
                            row.col(|ui| {
                                let size_str = if data.is_directory {
                                    String::new()
                                } else {
                                    crate::format_size(data.file_size)
                                };
                                if ui.selectable_label(is_selected, &size_str).clicked() {
                                    new_selection = Some(row_index);
                                }
                            });
                            row.col(|ui| {
                                let date_str = if data.modification_time > 0 {
                                    crate::format_filetime(data.modification_time)
                                } else {
                                    String::new()
                                };
                                if ui.selectable_label(is_selected, &date_str).clicked() {
                                    new_selection = Some(row_index);
                                }
                            });
                        }
                    }
                });
            });

        if let Some(idx) = new_selection {
            self.table.selected = Some(idx);
        }

        if let Some(column) = clicked_column {
            self.handle_sort_click(column);
        }
    }

    /// Handle sort column click - uses reverse optimization when possible
    fn handle_sort_click(&mut self, column: SortColumn) {
        if self.is_sorting {
            return; // Don't start another sort while one is in progress
        }

        let new_order = if self.table.sort_column == column {
            // Same column - toggle order
            if self.table.sort_order == SortOrder::Ascending {
                SortOrder::Descending
            } else {
                SortOrder::Ascending
            }
        } else {
            SortOrder::Ascending
        };

        // Check if we can use reverse optimization
        if self.last_sort_column == Some(column) && self.last_sort_order != new_order {
            // Same column, different order - just reverse!
            self.filtered_indices.reverse();
            self.table.sort_column = column;
            self.table.sort_order = new_order;
            self.last_sort_order = new_order;
            return;
        }

        // Need to do a full sort - do it in background
        self.table.sort_column = column;
        self.table.sort_order = new_order;
        self.is_sorting = true;

        // Clone data for background thread
        let mut indices = self.filtered_indices.clone();
        let entries = self.all_entries.clone();
        let sort_column = column;
        let sort_order = new_order;

        if let Some(tx) = &self.sort_sender {
            let tx = tx.clone();
            thread::spawn(move || {
                let _ = tx.send(BackgroundMessage::SortProgress("Sorting...".to_string()));

                // Sort using cached sort keys
                indices.sort_by(|&a, &b| {
                    let ea = &entries[a];
                    let eb = &entries[b];

                    let cmp = match sort_column {
                        SortColumn::Name => ea.name_lower.cmp(&eb.name_lower),
                        SortColumn::Path => std::cmp::Ordering::Equal, // Skip path sorting
                        SortColumn::Size => ea.file_size.cmp(&eb.file_size),
                        SortColumn::DateModified => ea.modification_time.cmp(&eb.modification_time),
                    };

                    if sort_order == SortOrder::Descending {
                        cmp.reverse()
                    } else {
                        cmp
                    }
                });

                let _ = tx.send(BackgroundMessage::SortComplete(sort_column, indices));
            });
        }
    }
}
