//! File Tree Management
//!
//! Builds and manages the hierarchical file tree from MFT/USN data.
//! Supports path resolution, size aggregation, and efficient traversal.

use crate::ntfs::{FileEntry, UsnEntry};
use crate::ntfs::mft::{extract_parent_info, extract_parent_info_debug};
use crate::ntfs::winapi::{get_ntfs_file_record, open_volume, SafeHandle};
use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// Tree Node
// ============================================================================

/// A node in the file tree (file or directory)
#[derive(Debug, Clone, Default)]
pub struct TreeNode {
    /// MFT record number (unique identifier)
    pub record_number: u64,
    /// Parent record number (5 = root)
    pub parent_record_number: u64,
    /// File/directory name
    pub name: String,
    /// File size in bytes (0 for directories)
    pub file_size: u64,
    /// Allocated size on disk
    pub allocated_size: u64,
    /// File attributes
    pub attributes: u32,
    /// Is this a directory?
    pub is_directory: bool,
    /// Creation time (FILETIME)
    pub creation_time: u64,
    /// Modification time (FILETIME)
    pub modification_time: u64,
    /// Children (for directories) - record numbers
    pub children: Vec<u64>,
    /// Aggregated size (self + all descendants)
    pub total_size: u64,
    /// Aggregated allocated size
    pub total_allocated: u64,
    /// Number of files in subtree (including self if file)
    pub file_count: u64,
    /// Number of directories in subtree (including self if directory)
    pub dir_count: u64,
}

impl TreeNode {
    /// Create from FileEntry
    pub fn from_file_entry(entry: &FileEntry) -> Self {
        Self {
            record_number: entry.record_number,
            parent_record_number: entry.parent_record_number,
            name: entry.name.clone(),
            file_size: entry.file_size,
            allocated_size: entry.allocated_size,
            attributes: entry.attributes,
            is_directory: entry.is_directory,
            creation_time: entry.creation_time,
            modification_time: entry.modification_time,
            children: Vec::new(),
            total_size: entry.file_size,
            total_allocated: entry.allocated_size,
            file_count: if entry.is_directory { 0 } else { 1 },
            dir_count: if entry.is_directory { 1 } else { 0 },
        }
    }

    /// Create from UsnEntry (lightweight, no size info)
    pub fn from_usn_entry(entry: &UsnEntry) -> Self {
        Self {
            record_number: entry.record_number,
            parent_record_number: entry.parent_record_number,
            name: entry.name.clone(),
            file_size: 0,
            allocated_size: 0,
            attributes: entry.attributes,
            is_directory: entry.is_directory,
            creation_time: 0,
            modification_time: 0,
            children: Vec::new(),
            total_size: 0,
            total_allocated: 0,
            file_count: if entry.is_directory { 0 } else { 1 },
            dir_count: if entry.is_directory { 1 } else { 0 },
        }
    }

    /// Update with size information from MFT
    pub fn update_from_file_entry(&mut self, entry: &FileEntry) {
        self.file_size = entry.file_size;
        self.allocated_size = entry.allocated_size;
        self.total_size = entry.file_size;
        self.total_allocated = entry.allocated_size;
        self.creation_time = entry.creation_time;
        self.modification_time = entry.modification_time;
    }
}

// ============================================================================
// File Tree
// ============================================================================

/// The complete file tree for a volume
pub struct FileTree {
    /// Drive letter
    pub drive_letter: char,
    /// All nodes indexed by record number
    nodes: DashMap<u64, TreeNode>,
    /// Root record number (typically 5)
    root_record: u64,
    /// Statistics
    pub stats: TreeStats,
    /// Bytes per MFT record (for on-demand parent resolution)
    bytes_per_record: u32,
}

/// Statistics about the tree
#[derive(Debug, Clone, Default)]
pub struct TreeStats {
    pub total_files: u64,
    pub total_directories: u64,
    pub total_size: u64,
    pub total_allocated: u64,
    pub orphaned_files: u64,
    pub max_depth: u32,
}

impl FileTree {
    /// Create a new empty file tree
    pub fn new(drive_letter: char) -> Self {
        Self {
            drive_letter,
            nodes: DashMap::new(),
            root_record: 5, // NTFS root is always record 5
            stats: TreeStats::default(),
            bytes_per_record: 1024, // Default MFT record size
        }
    }

    /// Create a new file tree with volume info for on-demand parent resolution
    pub fn with_volume_info(drive_letter: char, bytes_per_record: u32) -> Self {
        Self {
            drive_letter,
            nodes: DashMap::new(),
            root_record: 5,
            stats: TreeStats::default(),
            bytes_per_record,
        }
    }

    /// Set bytes per record (for on-demand parent resolution)
    pub fn set_bytes_per_record(&mut self, bytes_per_record: u32) {
        self.bytes_per_record = bytes_per_record;
    }

    /// Insert a node into the tree
    pub fn insert(&self, node: TreeNode) {
        let is_dir = node.is_directory;
        let size = node.file_size;
        let allocated = node.allocated_size;
        let record_num = node.record_number;
        let parent = node.parent_record_number;

        self.nodes.insert(record_num, node);

        // Update parent's children list
        if let Some(mut parent_node) = self.nodes.get_mut(&parent) {
            if !parent_node.children.contains(&record_num) {
                parent_node.children.push(record_num);
            }
        }

        // Update stats
        if is_dir {
            // Use atomic operations in production; simplified here
        } else {
            // Use atomic operations in production
        }
    }

    /// Get a node by record number
    pub fn get(&self, record_number: u64) -> Option<TreeNode> {
        self.nodes.get(&record_number).map(|r| r.clone())
    }

    /// Get children of a directory
    pub fn get_children(&self, record_number: u64) -> Vec<TreeNode> {
        if let Some(node) = self.nodes.get(&record_number) {
            node.children
                .iter()
                .filter_map(|&child_id| self.get(child_id))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get the root node
    pub fn root(&self) -> Option<TreeNode> {
        self.get(self.root_record)
    }

    /// Build full path for a record
    ///
    /// This method walks up the parent chain to construct the full path.
    /// If a parent is missing from the tree, it will attempt to fetch it
    /// on-demand using FSCTL_GET_NTFS_FILE_RECORD and cache it for future use.
    pub fn build_path(&self, record_number: u64) -> String {
        self.build_path_internal(record_number, false)
    }

    /// Build path with optional debug output
    pub fn build_path_debug(&self, record_number: u64) -> String {
        self.build_path_internal(record_number, true)
    }

    fn build_path_internal(&self, record_number: u64, debug: bool) -> String {
        let mut parts = Vec::new();
        let mut current = record_number;

        // Lazily open volume handle only if we encounter a missing parent
        let mut volume_handle: Option<SafeHandle> = None;

        while current != self.root_record && current != 0 {
            if let Some(node) = self.nodes.get(&current) {
                if debug {
                    eprintln!("  [path] Record {} '{}' -> parent {}", current, node.name, node.parent_record_number);
                }
                parts.push(node.name.clone());
                current = node.parent_record_number;
            } else {
                if debug {
                    eprintln!("  [path] Record {} NOT in tree, attempting fetch...", current);
                }
                // Parent not in tree - try to fetch it on-demand
                if volume_handle.is_none() {
                    match open_volume(self.drive_letter) {
                        Ok(h) => {
                            if debug {
                                eprintln!("  [path] Opened volume {}:", self.drive_letter);
                            }
                            volume_handle = Some(h);
                        }
                        Err(e) => {
                            if debug {
                                eprintln!("  [path] Failed to open volume: {}", e);
                            }
                            break;
                        }
                    }
                }

                if let Some(ref handle) = volume_handle {
                    if debug {
                        eprintln!("  [path] Calling get_ntfs_file_record({}, bytes={})", current, self.bytes_per_record);
                    }
                    match get_ntfs_file_record(handle, current, self.bytes_per_record) {
                        Ok(data) => {
                            if debug {
                                eprintln!("  [path] Got {} bytes of MFT data", data.len());
                                if data.len() >= 4 {
                                    eprintln!("  [path] Signature: {:02X} {:02X} {:02X} {:02X}",
                                        data[0], data[1], data[2], data[3]);
                                }
                            }
                            let parse_result = if debug {
                                extract_parent_info_debug(&data)
                            } else {
                                extract_parent_info(&data)
                            };
                            match parse_result {
                                Some((name, parent)) => {
                                    if debug {
                                        eprintln!("  [path] Extracted: name='{}', parent={}", name, parent);
                                    }
                                    // Cache this record for future lookups
                                    let node = TreeNode {
                                        record_number: current,
                                        parent_record_number: parent,
                                        name: name.clone(),
                                        is_directory: true,
                                        ..Default::default()
                                    };
                                    self.nodes.insert(current, node);

                                    parts.push(name);
                                    current = parent;
                                    continue;
                                }
                                None => {
                                    if debug {
                                        eprintln!("  [path] extract_parent_info returned None");
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            if debug {
                                eprintln!("  [path] get_ntfs_file_record failed: {}", e);
                            }
                        }
                    }
                }
                // Failed to fetch parent - stop here
                break;
            }
        }

        parts.reverse();
        format!("{}:\\{}", self.drive_letter, parts.join("\\"))
    }

    /// Calculate aggregated sizes (call after all nodes inserted)
    /// Uses iterative post-order traversal to avoid stack overflow
    pub fn calculate_sizes(&self) {
        use std::collections::HashMap;
        
        // We need to process children before parents (post-order)
        // Use iterative approach with explicit stack to avoid stack overflow
        
        // First pass: collect all nodes and their children in topological order
        let mut to_visit = vec![self.root_record];
        let mut visit_order = Vec::new();
        let mut visited = std::collections::HashSet::new();
        
        while let Some(record) = to_visit.pop() {
            if visited.contains(&record) {
                continue;
            }
            visited.insert(record);
            visit_order.push(record);
            
            if let Some(node) = self.nodes.get(&record) {
                for &child in &node.children {
                    if !visited.contains(&child) {
                        to_visit.push(child);
                    }
                }
            }
        }
        
        // Second pass: process in reverse order (leaves first)
        // Store computed values in a separate map to avoid holding refs
        let mut computed: HashMap<u64, (u64, u64, u64, u64)> = HashMap::new();
        
        for &record in visit_order.iter().rev() {
            let (children, file_size, allocated_size, is_directory) = {
                if let Some(node) = self.nodes.get(&record) {
                    (node.children.clone(), node.file_size, node.allocated_size, node.is_directory)
                } else {
                    continue;
                }
            };
            
            let mut total_size = file_size;
            let mut total_allocated = allocated_size;
            let mut file_count = if is_directory { 0 } else { 1 };
            let mut dir_count = if is_directory { 1 } else { 0 };
            
            // Sum up children's computed values
            for child_id in children {
                if let Some(&(cs, ca, fc, dc)) = computed.get(&child_id) {
                    total_size += cs;
                    total_allocated += ca;
                    file_count += fc;
                    dir_count += dc;
                }
            }
            
            computed.insert(record, (total_size, total_allocated, file_count, dir_count));
            
            // Update the node
            if let Some(mut node) = self.nodes.get_mut(&record) {
                node.total_size = total_size;
                node.total_allocated = total_allocated;
                node.file_count = file_count;
                node.dir_count = dir_count;
            }
        }
    }

    /// Get total number of nodes
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if tree is empty
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Iterate over all nodes
    pub fn iter(&self) -> impl Iterator<Item = dashmap::mapref::multiple::RefMulti<u64, TreeNode>> + '_ {
        self.nodes.iter()
    }

    /// Find orphaned nodes (no valid parent)
    pub fn find_orphans(&self) -> Vec<u64> {
        let mut orphans = Vec::new();

        for entry in self.nodes.iter() {
            let node = entry.value();
            if node.parent_record_number != 0
                && node.parent_record_number != self.root_record
                && !self.nodes.contains_key(&node.parent_record_number)
            {
                orphans.push(node.record_number);
            }
        }

        orphans
    }

    /// Compute final statistics
    pub fn compute_stats(&mut self) {
        let mut stats = TreeStats::default();

        for entry in self.nodes.iter() {
            let node = entry.value();
            if node.is_directory {
                stats.total_directories += 1;
            } else {
                stats.total_files += 1;
                stats.total_size += node.file_size;
                stats.total_allocated += node.allocated_size;
            }
        }

        stats.orphaned_files = self.find_orphans().len() as u64;
        self.stats = stats;
    }
}

// ============================================================================
// Tree Builder
// ============================================================================

/// Builds a FileTree from various sources
pub struct TreeBuilder {
    tree: FileTree,
}

impl TreeBuilder {
    /// Create a new builder
    pub fn new(drive_letter: char) -> Self {
        Self {
            tree: FileTree::new(drive_letter),
        }
    }

    /// Create a builder with volume info for on-demand parent resolution
    pub fn with_volume_info(drive_letter: char, bytes_per_record: u32) -> Self {
        Self {
            tree: FileTree::with_volume_info(drive_letter, bytes_per_record),
        }
    }

    /// Add entries from USN enumeration
    pub fn add_usn_entries(&mut self, entries: impl Iterator<Item = UsnEntry>) {
        for entry in entries {
            let node = TreeNode::from_usn_entry(&entry);
            self.tree.insert(node);
        }
    }

    /// Add entries from MFT parsing
    pub fn add_file_entries(&mut self, entries: impl Iterator<Item = FileEntry>) {
        for entry in entries {
            if !entry.is_valid {
                continue;
            }

            // Check if node exists (from USN) and update, or create new
            if let Some(mut existing) = self.tree.nodes.get_mut(&entry.record_number) {
                existing.update_from_file_entry(&entry);
            } else {
                let node = TreeNode::from_file_entry(&entry);
                self.tree.insert(node);
            }
        }
    }

    /// Finalize the tree
    pub fn build(mut self) -> FileTree {
        // Link children to parents
        self.link_children();

        // Calculate aggregated sizes
        self.tree.calculate_sizes();

        // Compute statistics
        self.tree.compute_stats();

        self.tree
    }

    /// Ensure all children are linked to parents
    fn link_children(&mut self) {
        // Collect all (child, parent) pairs
        let pairs: Vec<(u64, u64)> = self
            .tree
            .nodes
            .iter()
            .map(|e| (e.record_number, e.parent_record_number))
            .collect();

        // Link children
        for (child_id, parent_id) in pairs {
            if let Some(mut parent) = self.tree.nodes.get_mut(&parent_id) {
                if !parent.children.contains(&child_id) {
                    parent.children.push(child_id);
                }
            }
        }
    }
}

// ============================================================================
// Search Support
// ============================================================================

/// Search result entry
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub record_number: u64,
    pub name: String,
    pub path: String,
    pub file_size: u64,
    pub is_directory: bool,
    pub modification_time: u64,
}

impl FileTree {
    /// Search for files matching a pattern
    pub fn search(&self, pattern: &str, max_results: usize) -> Vec<SearchResult> {
        let pattern_lower = pattern.to_lowercase();
        let mut results = Vec::new();

        for entry in self.nodes.iter() {
            if results.len() >= max_results {
                break;
            }

            let node = entry.value();
            if node.name.to_lowercase().contains(&pattern_lower) {
                results.push(SearchResult {
                    record_number: node.record_number,
                    name: node.name.clone(),
                    path: self.build_path(node.record_number),
                    file_size: node.file_size,
                    is_directory: node.is_directory,
                    modification_time: node.modification_time,
                });
            }
        }

        results
    }

    /// Search with regex (requires regex crate)
    pub fn search_glob(&self, pattern: &str, max_results: usize) -> Vec<SearchResult> {
        // Simple glob-to-contains conversion
        let search_term = pattern
            .replace("*", "")
            .replace("?", "")
            .to_lowercase();

        self.search(&search_term, max_results)
    }

    /// Get largest files
    pub fn largest_files(&self, count: usize) -> Vec<SearchResult> {
        let mut files: Vec<_> = self
            .nodes
            .iter()
            .filter(|e| !e.value().is_directory)
            .map(|e| {
                let node = e.value();
                (node.record_number, node.file_size)
            })
            .collect();

        files.sort_by(|a, b| b.1.cmp(&a.1));
        files.truncate(count);

        files
            .into_iter()
            .filter_map(|(record_num, _)| {
                self.get(record_num).map(|node| SearchResult {
                    record_number: node.record_number,
                    name: node.name.clone(),
                    path: self.build_path(node.record_number),
                    file_size: node.file_size,
                    is_directory: node.is_directory,
                    modification_time: node.modification_time,
                })
            })
            .collect()
    }

    /// Get largest directories by total size
    pub fn largest_directories(&self, count: usize) -> Vec<SearchResult> {
        let mut dirs: Vec<_> = self
            .nodes
            .iter()
            .filter(|e| e.value().is_directory)
            .map(|e| {
                let node = e.value();
                (node.record_number, node.total_size)
            })
            .collect();

        dirs.sort_by(|a, b| b.1.cmp(&a.1));
        dirs.truncate(count);

        dirs.into_iter()
            .filter_map(|(record_num, _)| {
                self.get(record_num).map(|node| SearchResult {
                    record_number: node.record_number,
                    name: node.name.clone(),
                    path: self.build_path(node.record_number),
                    file_size: node.total_size,
                    is_directory: node.is_directory,
                    modification_time: node.modification_time,
                })
            })
            .collect()
    }
}
