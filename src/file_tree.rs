//! File Tree Management
//!
//! Builds and manages the hierarchical file tree from MFT/USN data.
//! Supports path resolution, size aggregation, and efficient traversal.
//!
//! # Hard Link Support
//!
//! NTFS supports hard links - multiple directory entries pointing to the same file.
//! Each hard link has a different parent directory but shares the same MFT record.
//! To properly handle this, we use a composite key `NodeKey(record_number, parent_record_number)`
//! which allows multiple entries for the same file with different parents.

use crate::logging;
use crate::ntfs::{FileEntry, UsnEntry};
use crate::ntfs::mft::{extract_parent_info, extract_parent_info_debug};
use crate::ntfs::physical::MftRecordFetcher;
use crate::ntfs::winapi::{get_ntfs_file_record, open_volume, open_volume_for_file_id, SafeHandle};
use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// Node Key - Composite key for hard link support
// ============================================================================

/// Composite key for tree nodes: (record_number, parent_record_number)
///
/// This allows multiple entries for the same MFT record with different parents,
/// which is necessary for proper hard link support. Each hard link appears as
/// a separate entry in the file tree with its own parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NodeKey {
    /// MFT record number
    pub record_number: u64,
    /// Parent directory record number
    pub parent_record_number: u64,
}

impl NodeKey {
    pub fn new(record_number: u64, parent_record_number: u64) -> Self {
        Self { record_number, parent_record_number }
    }

    /// Create key for root node (record 5, parent 5)
    pub fn root() -> Self {
        Self { record_number: 5, parent_record_number: 5 }
    }
}

// ============================================================================
// Tree Node
// ============================================================================

/// A node in the file tree (file or directory)
#[derive(Debug, Clone, Default)]
pub struct TreeNode {
    /// MFT record number
    pub record_number: u64,
    /// Parent record number (5 = root)
    pub parent_record_number: u64,
    /// Full file reference number including sequence (for OpenFileById)
    pub file_reference_number: u64,
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
    /// Children (for directories) - NodeKeys of children
    pub children: Vec<NodeKey>,
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
            // Use full FRN from FileEntry (includes sequence number for OpenFileById)
            file_reference_number: entry.file_reference_number,
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
            file_reference_number: entry.file_reference_number,
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
        // Update file_reference_number if MFT provides a valid one
        // (MFT's FRN includes sequence number which is more accurate)
        if entry.file_reference_number != 0 {
            self.file_reference_number = entry.file_reference_number;
        }
    }

    /// Get the NodeKey for this node
    #[inline]
    pub fn key(&self) -> NodeKey {
        NodeKey::new(self.record_number, self.parent_record_number)
    }

    /// Create a TreeNode for a hard link (same file, different parent)
    pub fn from_hard_link(entry: &FileEntry, link: &crate::ntfs::HardLink) -> Self {
        Self {
            record_number: entry.record_number,
            parent_record_number: link.parent_record_number,
            // Use full FRN from FileEntry (includes sequence number for OpenFileById)
            file_reference_number: entry.file_reference_number,
            name: link.name.clone(),
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
}

// ============================================================================
// File Tree
// ============================================================================

/// The complete file tree for a volume
pub struct FileTree {
    /// Drive letter
    pub drive_letter: char,
    /// All nodes indexed by NodeKey (record_number, parent_record_number)
    /// This composite key allows multiple entries for the same file (hard links)
    pub(crate) nodes: DashMap<NodeKey, TreeNode>,
    /// Secondary index: record_number -> Vec<NodeKey>
    /// Allows quick lookup of all hard links for a given record
    record_index: DashMap<u64, Vec<NodeKey>>,
    /// Name index: (parent_record_number, name_lowercase) -> NodeKey
    /// Used to deduplicate files with the same parent and name (like Everything does)
    name_index: DashMap<(u64, String), NodeKey>,
    /// Root record number (typically 5)
    root_record: u64,
    /// Statistics
    pub stats: TreeStats,
    /// Bytes per MFT record (for on-demand parent resolution)
    bytes_per_record: u32,
    /// MFT record fetcher for on-demand parent resolution (replaces FSCTL_GET_NTFS_FILE_RECORD)
    record_fetcher: Option<Arc<MftRecordFetcher>>,
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
            record_index: DashMap::new(),
            name_index: DashMap::new(),
            root_record: 5, // NTFS root is always record 5
            stats: TreeStats::default(),
            bytes_per_record: 1024, // Default MFT record size
            record_fetcher: None,
        }
    }

    /// Create a new file tree with volume info for on-demand parent resolution
    pub fn with_volume_info(drive_letter: char, bytes_per_record: u32) -> Self {
        Self {
            drive_letter,
            nodes: DashMap::new(),
            record_index: DashMap::new(),
            name_index: DashMap::new(),
            root_record: 5,
            stats: TreeStats::default(),
            bytes_per_record,
            record_fetcher: None,
        }
    }

    /// Set the MFT record fetcher for on-demand parent resolution
    pub fn set_record_fetcher(&mut self, fetcher: Arc<MftRecordFetcher>) {
        self.record_fetcher = Some(fetcher);
    }

    /// Set bytes per record (for on-demand parent resolution)
    pub fn set_bytes_per_record(&mut self, bytes_per_record: u32) {
        self.bytes_per_record = bytes_per_record;
    }

    /// Insert a node into the tree
    /// Returns true if the node was inserted, false if a duplicate (same parent+name) already exists
    pub fn insert(&self, node: TreeNode) -> bool {
        let key = node.key();
        let name_key = (key.parent_record_number, node.name.to_lowercase());

        // Check if a file with the same parent and name already exists (like Everything does)
        // This deduplicates entries that have the same path but different record numbers
        // (e.g., from multiple $FILE_NAME attributes with different namespaces)
        if self.name_index.contains_key(&name_key) {
            return false; // Duplicate path - skip
        }

        // Insert into name index first
        self.name_index.insert(name_key, key);

        // Insert into main map
        self.nodes.insert(key, node);

        // Update secondary index
        self.record_index
            .entry(key.record_number)
            .or_insert_with(Vec::new)
            .push(key);

        // Update parent's children list (find parent by record_number)
        // We look up any node with the parent's record number
        if let Some(parent_keys) = self.record_index.get(&key.parent_record_number) {
            for parent_key in parent_keys.iter() {
                if let Some(mut parent_node) = self.nodes.get_mut(parent_key) {
                    if !parent_node.children.contains(&key) {
                        parent_node.children.push(key);
                    }
                    break; // Only need to add to one parent
                }
            }
        }

        true
    }

    /// Get a node by NodeKey
    pub fn get_by_key(&self, key: &NodeKey) -> Option<TreeNode> {
        self.nodes.get(key).map(|r| r.clone())
    }

    /// Get the first node for a record number (any parent)
    /// Used when you don't care which hard link you get
    pub fn get(&self, record_number: u64) -> Option<TreeNode> {
        if let Some(keys) = self.record_index.get(&record_number) {
            if let Some(first_key) = keys.first() {
                return self.nodes.get(first_key).map(|r| r.clone());
            }
        }
        None
    }

    /// Get all nodes for a record number (all hard links)
    pub fn get_all(&self, record_number: u64) -> Vec<TreeNode> {
        if let Some(keys) = self.record_index.get(&record_number) {
            keys.iter()
                .filter_map(|key| self.nodes.get(key).map(|r| r.clone()))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get children of a directory
    pub fn get_children(&self, key: &NodeKey) -> Vec<TreeNode> {
        if let Some(node) = self.nodes.get(key) {
            node.children
                .iter()
                .filter_map(|child_key| self.get_by_key(child_key))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get the root node
    pub fn root(&self) -> Option<TreeNode> {
        // Root has record 5 and parent 5
        self.get_by_key(&NodeKey::root())
    }

    /// Build full path for a NodeKey
    ///
    /// This method walks up the parent chain to construct the full path.
    /// If a parent is missing from the tree, it will attempt to fetch it
    /// on-demand using FSCTL_GET_NTFS_FILE_RECORD and cache it for future use.
    pub fn build_path_for_key(&self, key: &NodeKey) -> String {
        self.build_path_internal_key(key, false)
    }

    /// Build full path for a record number (uses first available hard link)
    pub fn build_path(&self, record_number: u64) -> String {
        if let Some(keys) = self.record_index.get(&record_number) {
            if let Some(first_key) = keys.first() {
                return self.build_path_for_key(first_key);
            }
        }
        // Fallback: try to build path walking up by record number only
        self.build_path_internal(record_number, false)
    }

    /// Build path with optional debug output
    pub fn build_path_debug(&self, record_number: u64) -> String {
        self.build_path_internal(record_number, true)
    }

    /// Build path for a NodeKey with optional debug output
    fn build_path_internal_key(&self, start_key: &NodeKey, debug: bool) -> String {
        let mut parts = Vec::new();

        // First, get the starting node's name
        if let Some(node) = self.nodes.get(start_key) {
            if debug {
                eprintln!("  [path] Start: Record {} '{}' -> parent {}",
                    start_key.record_number, node.name, node.parent_record_number);
            }
            parts.push(node.name.clone());

            // Now walk up the parent chain
            let mut current_parent = node.parent_record_number;
            self.walk_parent_chain(&mut parts, current_parent, debug);
        }

        parts.reverse();
        format!("{}:\\{}", self.drive_letter, parts.join("\\"))
    }

    /// Walk up the parent chain collecting path components
    fn walk_parent_chain(&self, parts: &mut Vec<String>, start_parent: u64, debug: bool) {
        let mut current = start_parent;
        let mut volume_handle: Option<SafeHandle> = None;

        while current != self.root_record && current != 0 {
            // Find any node with this record number (directories have only one entry)
            if let Some(node) = self.get(current) {
                if debug {
                    eprintln!("  [path] Record {} '{}' -> parent {}",
                        current, node.name, node.parent_record_number);
                }
                parts.push(node.name.clone());
                current = node.parent_record_number;
            } else {
                if debug {
                    eprintln!("  [path] Record {} NOT in tree, attempting fetch...", current);
                }

                // Try MftRecordFetcher first (direct disk read, works in physical mode)
                if let Some(ref fetcher) = self.record_fetcher {
                    if debug {
                        eprintln!("  [path] Using MftRecordFetcher for record {}", current);
                    }
                    if let Some((name, parent)) = fetcher.fetch_parent_info(current) {
                        if debug {
                            eprintln!("  [path] Fetcher got: name='{}', parent={}", name, parent);
                        }
                        let node = TreeNode {
                            record_number: current,
                            parent_record_number: parent,
                            name: name.clone(),
                            is_directory: true,
                            ..Default::default()
                        };
                        self.insert(node);
                        parts.push(name);
                        current = parent;
                        continue;
                    }
                    if debug {
                        eprintln!("  [path] MftRecordFetcher returned None");
                    }
                    break;
                }

                // Fallback: use FSCTL_GET_NTFS_FILE_RECORD (volume mode only)
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
                        eprintln!("  [path] Calling get_ntfs_file_record({}, bytes={})",
                            current, self.bytes_per_record);
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
                                    let node = TreeNode {
                                        record_number: current,
                                        parent_record_number: parent,
                                        name: name.clone(),
                                        is_directory: true,
                                        ..Default::default()
                                    };
                                    self.insert(node);

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
    }

    fn build_path_internal(&self, record_number: u64, debug: bool) -> String {
        let mut parts = Vec::new();
        let mut current = record_number;

        // Lazily open volume handle only if we encounter a missing parent
        let mut volume_handle: Option<SafeHandle> = None;

        while current != self.root_record && current != 0 {
            if let Some(node) = self.get(current) {
                if debug {
                    eprintln!("  [path] Record {} '{}' -> parent {}", current, node.name, node.parent_record_number);
                }
                parts.push(node.name.clone());
                current = node.parent_record_number;
            } else {
                if debug {
                    eprintln!("  [path] Record {} NOT in tree, attempting fetch...", current);
                }

                // Try MftRecordFetcher first (direct disk read, works in physical mode)
                if let Some(ref fetcher) = self.record_fetcher {
                    if debug {
                        eprintln!("  [path] Using MftRecordFetcher for record {}", current);
                    }
                    if let Some((name, parent)) = fetcher.fetch_parent_info(current) {
                        if debug {
                            eprintln!("  [path] Fetcher got: name='{}', parent={}", name, parent);
                        }
                        let node = TreeNode {
                            record_number: current,
                            parent_record_number: parent,
                            name: name.clone(),
                            is_directory: true,
                            ..Default::default()
                        };
                        self.insert(node);
                        parts.push(name);
                        current = parent;
                        continue;
                    }
                    if debug {
                        eprintln!("  [path] MftRecordFetcher returned None");
                    }
                    break;
                }

                // Fallback: use FSCTL_GET_NTFS_FILE_RECORD (volume mode only)
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
                                    let node = TreeNode {
                                        record_number: current,
                                        parent_record_number: parent,
                                        name: name.clone(),
                                        is_directory: true,
                                        ..Default::default()
                                    };
                                    self.insert(node);

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
        let root_key = NodeKey::root();
        let mut to_visit = vec![root_key];
        let mut visit_order = Vec::new();
        let mut visited = std::collections::HashSet::new();

        while let Some(key) = to_visit.pop() {
            if visited.contains(&key) {
                continue;
            }
            visited.insert(key);
            visit_order.push(key);

            if let Some(node) = self.nodes.get(&key) {
                for &child_key in &node.children {
                    if !visited.contains(&child_key) {
                        to_visit.push(child_key);
                    }
                }
            }
        }

        // Second pass: process in reverse order (leaves first)
        // Store computed values in a separate map to avoid holding refs
        let mut computed: HashMap<NodeKey, (u64, u64, u64, u64)> = HashMap::new();

        for &key in visit_order.iter().rev() {
            let (children, file_size, allocated_size, is_directory) = {
                if let Some(node) = self.nodes.get(&key) {
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
            for child_key in children {
                if let Some(&(cs, ca, fc, dc)) = computed.get(&child_key) {
                    total_size += cs;
                    total_allocated += ca;
                    file_count += fc;
                    dir_count += dc;
                }
            }

            computed.insert(key, (total_size, total_allocated, file_count, dir_count));

            // Update the node
            if let Some(mut node) = self.nodes.get_mut(&key) {
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
    pub fn iter(&self) -> impl Iterator<Item = dashmap::mapref::multiple::RefMulti<NodeKey, TreeNode>> + '_ {
        self.nodes.iter()
    }

    /// Find orphaned nodes (no valid parent)
    pub fn find_orphans(&self) -> Vec<NodeKey> {
        let mut orphans = Vec::new();

        for entry in self.nodes.iter() {
            let node = entry.value();
            // Check if parent exists in the record index
            if node.parent_record_number != 0
                && node.parent_record_number != self.root_record
                && !self.record_index.contains_key(&node.parent_record_number)
            {
                orphans.push(node.key());
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

    /// Refresh metadata for multiple files using Windows API
    ///
    /// This retrieves accurate file sizes and timestamps by opening each file
    /// by its File Reference Number and querying Windows directly. This is more
    /// accurate than MFT data for certain files (sparse files, hardlinks,
    /// files managed by filter drivers, etc.)
    ///
    /// Takes a slice of (NodeKey, file_reference_number) pairs.
    /// Returns a HashMap of NodeKey -> (file_size, modification_time) for successful updates.
    ///
    /// When metadata is refreshed for a file, ALL nodes with the same record_number
    /// (i.e., all hardlinks) are also updated to ensure consistency.
    pub fn refresh_metadata(&self, entries: &[(NodeKey, u64)]) -> std::collections::HashMap<NodeKey, (u64, u64)> {
        use crate::ntfs::winapi::get_file_metadata_by_id;

        let mut results = std::collections::HashMap::new();
        // Track which record_numbers we've already refreshed to avoid redundant API calls
        let mut refreshed_records: std::collections::HashSet<u64> = std::collections::HashSet::new();

        // Open volume root directory handle for OpenFileById
        let volume_handle = match open_volume_for_file_id(self.drive_letter) {
            Ok(h) => h,
            Err(_) => return results,
        };

        // Fetch metadata for each file using full FRN
        for &(key, file_ref) in entries {
            // Skip if we already refreshed this record (via a different hardlink)
            if refreshed_records.contains(&key.record_number) {
                // Still return the result for the caller's key
                if let Some(node) = self.nodes.get(&key) {
                    results.insert(key, (node.file_size, node.modification_time));
                }
                continue;
            }

            // Use the full file reference number for OpenFileById
            match get_file_metadata_by_id(&volume_handle, file_ref) {
                Ok(metadata) => {
                    refreshed_records.insert(key.record_number);

                    // Update ALL nodes with the same record_number (all hardlinks)
                    if let Some(all_keys) = self.record_index.get(&key.record_number) {
                        for hardlink_key in all_keys.iter() {
                            if let Some(mut node) = self.nodes.get_mut(hardlink_key) {
                                node.file_size = metadata.file_size;
                                node.creation_time = metadata.creation_time;
                                node.modification_time = metadata.modification_time;
                                node.total_size = metadata.file_size;
                            }
                            // Return result for all hardlinks
                            results.insert(*hardlink_key, (metadata.file_size, metadata.modification_time));
                        }
                    } else {
                        // Fallback: just update the requested node
                        if let Some(mut node) = self.nodes.get_mut(&key) {
                            node.file_size = metadata.file_size;
                            node.creation_time = metadata.creation_time;
                            node.modification_time = metadata.modification_time;
                            node.total_size = metadata.file_size;
                        }
                        results.insert(key, (metadata.file_size, metadata.modification_time));
                    }
                }
                Err(_e) => {
                    // Silently skip failures - file may be deleted or inaccessible
                    // (Uncomment for debugging: eprintln!("[metadata] Failed for key {:?} (FRN 0x{:016X}): {}", key, file_ref, _e);)
                }
            }
        }

        results
    }

    /// Refresh metadata for a single file, returning updated values
    ///
    /// Returns Some((file_size, modification_time)) if successful, None otherwise.
    /// When metadata is refreshed, ALL nodes with the same record_number
    /// (i.e., all hardlinks) are also updated to ensure consistency.
    pub fn refresh_single_metadata(&self, key: &NodeKey) -> Option<(u64, u64)> {
        use crate::ntfs::winapi::get_file_metadata_by_id;

        // Get the node to retrieve its full file reference number
        let file_ref = self.nodes.get(key)?.file_reference_number;

        let volume_handle = open_volume_for_file_id(self.drive_letter).ok()?;
        let metadata = get_file_metadata_by_id(&volume_handle, file_ref).ok()?;

        // Update ALL nodes with the same record_number (all hardlinks)
        if let Some(all_keys) = self.record_index.get(&key.record_number) {
            for hardlink_key in all_keys.iter() {
                if let Some(mut node) = self.nodes.get_mut(hardlink_key) {
                    node.file_size = metadata.file_size;
                    node.creation_time = metadata.creation_time;
                    node.modification_time = metadata.modification_time;
                    node.total_size = metadata.file_size;
                }
            }
        } else {
            // Fallback: just update the requested node
            if let Some(mut node) = self.nodes.get_mut(key) {
                node.file_size = metadata.file_size;
                node.creation_time = metadata.creation_time;
                node.modification_time = metadata.modification_time;
                node.total_size = metadata.file_size;
            }
        }

        Some((metadata.file_size, metadata.modification_time))
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

    /// Set the MFT record fetcher for on-demand parent resolution
    pub fn set_record_fetcher(&mut self, fetcher: Arc<MftRecordFetcher>) {
        self.tree.set_record_fetcher(fetcher);
    }

    /// Add entries from USN enumeration
    pub fn add_usn_entries(&mut self, entries: impl Iterator<Item = UsnEntry>) {
        for entry in entries {
            let node = TreeNode::from_usn_entry(&entry);

            // Log tree node creation from USN
            logging::log_tree_node_create(
                node.record_number,
                node.parent_record_number,
                &node.name,
                node.file_size,
                node.modification_time,
                "USN",
            );

            self.tree.insert(node);
        }
    }

    /// Add entries from MFT parsing
    ///
    /// This method handles hard links by creating separate tree nodes for each
    /// $FILE_NAME attribute with a different parent directory. Each hard link
    /// is stored with a unique NodeKey (record_number, parent_record_number).
    ///
    /// Critically, after processing each MFT entry, we propagate metadata (size,
    /// timestamps) to ALL nodes with the same record_number. This ensures that
    /// hard links discovered via USN (which has no metadata) get updated with
    /// the actual metadata from MFT.
    pub fn add_file_entries(&mut self, entries: impl Iterator<Item = FileEntry>) {
        for entry in entries {
            if !entry.is_valid {
                continue;
            }

            // Primary key for this entry
            let primary_key = NodeKey::new(entry.record_number, entry.parent_record_number);

            // Check if this specific node already exists and update it
            if let Some(mut existing) = self.tree.nodes.get_mut(&primary_key) {
                // Log the update
                logging::log_tree_node_update(
                    existing.record_number,
                    existing.parent_record_number,
                    &existing.name,
                    existing.file_size,
                    entry.file_size,
                    existing.modification_time,
                    entry.modification_time,
                    "MFT_primary_update",
                );
                existing.update_from_file_entry(&entry);
            } else {
                // Create primary node
                let node = TreeNode::from_file_entry(&entry);
                logging::log_tree_node_create(
                    node.record_number,
                    node.parent_record_number,
                    &node.name,
                    node.file_size,
                    node.modification_time,
                    "MFT_primary_new",
                );
                self.tree.insert(node);
            }

            // Create additional nodes for hard links found in MFT's $FILE_NAME attributes
            for link in &entry.hard_links {
                // Skip if this is the same as the primary entry
                if link.parent_record_number == entry.parent_record_number
                    && link.name == entry.name {
                    continue;
                }

                let link_key = NodeKey::new(entry.record_number, link.parent_record_number);

                // Only create if doesn't exist - we'll update all nodes below
                if !self.tree.nodes.contains_key(&link_key) {
                    let link_node = TreeNode::from_hard_link(&entry, link);
                    logging::log_tree_node_create(
                        link_node.record_number,
                        link_node.parent_record_number,
                        &link_node.name,
                        link_node.file_size,
                        link_node.modification_time,
                        "MFT_hardlink_new",
                    );
                    self.tree.insert(link_node);
                }
            }

            // CRITICAL: Propagate metadata to ALL nodes with this record_number
            // This handles the case where USN discovered hard links that MFT's
            // $FILE_NAME attributes don't list (e.g., names in extension records
            // or different namespace discovery). All hard links share the same
            // file data, so they must have the same size and timestamps.
            if let Some(all_keys) = self.tree.record_index.get(&entry.record_number) {
                let keys: Vec<NodeKey> = all_keys.iter().copied().collect();
                drop(all_keys); // Release the lock before modifying nodes

                // Log all hardlinks we know about for this record
                let hardlink_info: Vec<(u64, u64, String)> = keys.iter()
                    .filter_map(|k| {
                        self.tree.nodes.get(k).map(|n| (n.record_number, n.parent_record_number, n.name.clone()))
                    })
                    .collect();
                logging::log_all_hardlinks_for_record(entry.record_number, &hardlink_info);

                for key in keys {
                    // Skip the primary key - already updated above
                    if key == primary_key {
                        continue;
                    }

                    if let Some(mut node) = self.tree.nodes.get_mut(&key) {
                        // Log propagation
                        logging::log_metadata_propagation(
                            entry.record_number,
                            entry.parent_record_number,
                            key.parent_record_number,
                            &node.name,
                            entry.file_size,
                            entry.modification_time,
                        );

                        // Propagate metadata from MFT entry to this hard link
                        node.file_size = entry.file_size;
                        node.allocated_size = entry.allocated_size;
                        node.total_size = entry.file_size;
                        node.total_allocated = entry.allocated_size;
                        node.creation_time = entry.creation_time;
                        node.modification_time = entry.modification_time;
                        if entry.file_reference_number != 0 {
                            node.file_reference_number = entry.file_reference_number;
                        }
                    }
                }
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
        // Collect all (child_key, parent_record_number) pairs
        let pairs: Vec<(NodeKey, u64)> = self
            .tree
            .nodes
            .iter()
            .map(|e| (*e.key(), e.parent_record_number))
            .collect();

        // Link children to parents
        // For each child, find the parent node(s) by record number
        for (child_key, parent_record) in pairs {
            // Look up all nodes with this parent record number
            if let Some(parent_keys) = self.tree.record_index.get(&parent_record) {
                // Add child to the first matching parent (directories typically have one entry)
                for parent_key in parent_keys.iter() {
                    if let Some(mut parent_node) = self.tree.nodes.get_mut(parent_key) {
                        if !parent_node.children.contains(&child_key) {
                            parent_node.children.push(child_key);
                        }
                        break; // Only add to one parent node
                    }
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
    /// Unique key for this result (record_number, parent_record_number)
    pub key: NodeKey,
    /// MFT record number (convenience accessor)
    pub record_number: u64,
    pub name: String,
    pub path: String,
    pub file_size: u64,
    pub is_directory: bool,
    pub modification_time: u64,
}

impl SearchResult {
    /// Create a SearchResult from a TreeNode and its path
    fn from_node(node: &TreeNode, path: String) -> Self {
        Self {
            key: node.key(),
            record_number: node.record_number,
            name: node.name.clone(),
            path,
            file_size: node.file_size,
            is_directory: node.is_directory,
            modification_time: node.modification_time,
        }
    }
}

impl FileTree {
    /// Search for files matching a pattern
    pub fn search(&self, pattern: &str, max_results: usize) -> Vec<SearchResult> {
        logging::separator(&format!("SEARCH: '{}'", pattern));
        let pattern_lower = pattern.to_lowercase();
        let mut results = Vec::new();

        for entry in self.nodes.iter() {
            if results.len() >= max_results {
                break;
            }

            let node = entry.value();
            let key = *entry.key();

            // Skip entries with no name (incomplete MFT records)
            if node.name.is_empty() {
                continue;
            }
            if node.name.to_lowercase().contains(&pattern_lower) {
                let path = self.build_path_for_key(&key);

                // Log each search result with full details
                logging::log_search_result(
                    results.len(),
                    node.record_number,
                    node.parent_record_number,
                    &node.name,
                    &path,
                    node.file_size,
                    node.modification_time,
                    node.file_reference_number,
                );

                results.push(SearchResult::from_node(node, path));
            }
        }

        logging::info("SEARCH", &format!("Found {} results for '{}'", results.len(), pattern));
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
            .filter(|e| !e.value().is_directory && !e.value().name.is_empty())
            .map(|e| {
                let key = *e.key();
                let node = e.value();
                (key, node.file_size)
            })
            .collect();

        files.sort_by(|a, b| b.1.cmp(&a.1));
        files.truncate(count);

        files
            .into_iter()
            .filter_map(|(key, _)| {
                self.get_by_key(&key).map(|node| {
                    let path = self.build_path_for_key(&key);
                    SearchResult::from_node(&node, path)
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
                let key = *e.key();
                let node = e.value();
                (key, node.total_size)
            })
            .collect();

        dirs.sort_by(|a, b| b.1.cmp(&a.1));
        dirs.truncate(count);

        dirs.into_iter()
            .filter_map(|(key, _)| {
                self.get_by_key(&key).map(|node| {
                    let path = self.build_path_for_key(&key);
                    // For directories, use total_size instead of file_size
                    SearchResult {
                        key,
                        record_number: node.record_number,
                        name: node.name.clone(),
                        path,
                        file_size: node.total_size,
                        is_directory: node.is_directory,
                        modification_time: node.modification_time,
                    }
                })
            })
            .collect()
    }
}
