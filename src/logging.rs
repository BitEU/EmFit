//! Comprehensive Logging Module for EmFit Debugging
//!
//! This module provides detailed logging for debugging file metadata issues,
//! particularly for hard-linked files in WinSxS directories.
//!
//! FILTER: Only logs entries matching the DEBUG_PATTERN filename.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// Global logger instance
static LOGGER: OnceLock<Mutex<EmFitLogger>> = OnceLock::new();

/// Debug filter pattern - ONLY log entries containing this string in their name
/// Set to the specific filename we're debugging
const DEBUG_PATTERN: &str = "ScheduleTime_80.contrast-white.png";

/// Log levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
        }
    }
}

/// Check if a name matches our debug filter
#[inline]
fn matches_filter(name: &str) -> bool {
    name.to_lowercase().contains(&DEBUG_PATTERN.to_lowercase())
}

/// Main logger struct
pub struct EmFitLogger {
    file: Option<File>,
    min_level: LogLevel,
}

impl EmFitLogger {
    /// Create a new logger
    fn new() -> Self {
        let log_path = Self::get_log_path();
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true) // Start fresh each run
            .open(&log_path)
            .ok();

        if file.is_some() {
            eprintln!("[EmFit] Logging to: {}", log_path.display());
            eprintln!("[EmFit] Filter pattern: '{}'", DEBUG_PATTERN);
        }

        Self {
            file,
            min_level: LogLevel::Debug,
        }
    }

    /// Get the log file path (same directory as executable)
    fn get_log_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("emfit.log")
    }

    /// Write a log entry
    fn log(&mut self, level: LogLevel, module: &str, message: &str) {
        if level < self.min_level {
            return;
        }

        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        let entry = format!(
            "[{:013}] [{:5}] [{}] {}\n",
            timestamp, level, module, message
        );

        if let Some(ref mut file) = self.file {
            let _ = file.write_all(entry.as_bytes());
            let _ = file.flush();
        }
    }
}

/// Initialize the global logger
pub fn init() {
    let _ = LOGGER.set(Mutex::new(EmFitLogger::new()));
}

/// Log a message (always logs, for INFO/WARN/ERROR and separators)
fn log(level: LogLevel, module: &str, message: &str) {
    if let Some(logger) = LOGGER.get() {
        if let Ok(mut l) = logger.lock() {
            l.log(level, module, message);
        }
    }
}

/// Log debug message
pub fn debug(module: &str, message: &str) {
    log(LogLevel::Debug, module, message);
}

/// Log info message
pub fn info(module: &str, message: &str) {
    log(LogLevel::Info, module, message);
}

/// Log warning message
pub fn warn(module: &str, message: &str) {
    log(LogLevel::Warn, module, message);
}

/// Log error message
pub fn error(module: &str, message: &str) {
    log(LogLevel::Error, module, message);
}

// ============================================================================
// Specialized logging functions for different components
// These ONLY log if the filename matches DEBUG_PATTERN
// ============================================================================

/// Log USN entry details - FILTERED by name
pub fn log_usn_entry(
    record_number: u64,
    parent_record_number: u64,
    file_reference_number: u64,
    name: &str,
    attributes: u32,
    is_directory: bool,
) {
    // Only log if name matches our filter
    if !matches_filter(name) {
        return;
    }

    let msg = format!(
        "USN Entry: record={}, parent={}, frn=0x{:016X}, name='{}', attrs=0x{:08X}, is_dir={}",
        record_number, parent_record_number, file_reference_number, name, attributes, is_directory
    );
    info("USN", &msg);
}

/// Log MFT entry details (full dump) - FILTERED by name
pub fn log_mft_entry(
    record_number: u64,
    parent_record_number: u64,
    file_reference_number: u64,
    name: &str,
    file_size: u64,
    allocated_size: u64,
    creation_time: u64,
    modification_time: u64,
    attributes: u32,
    is_directory: bool,
    hard_link_count: u16,
    extension_records: &[u64],
    data_extension_record: Option<u64>,
    hard_links: &[(u64, String)], // (parent, name) pairs
) {
    // Only log if name matches our filter
    if !matches_filter(name) {
        return;
    }

    let ext_recs = extension_records
        .iter()
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let links = hard_links
        .iter()
        .map(|(p, n)| format!("(parent={}, name='{}')", p, n))
        .collect::<Vec<_>>()
        .join(", ");

    let msg = format!(
        "MFT Entry: record={}, parent={}, frn=0x{:016X}, name='{}', \
         size={}, alloc={}, ctime={}, mtime={}, attrs=0x{:08X}, \
         is_dir={}, link_count={}, ext_records=[{}], data_ext={:?}, \
         hard_links=[{}]",
        record_number, parent_record_number, file_reference_number, name,
        file_size, allocated_size, creation_time, modification_time, attributes,
        is_directory, hard_link_count, ext_recs, data_extension_record, links
    );
    info("MFT", &msg);
}

/// Log tree node creation/update - FILTERED by name
pub fn log_tree_node_create(
    record_number: u64,
    parent_record_number: u64,
    name: &str,
    file_size: u64,
    modification_time: u64,
    source: &str, // "USN", "MFT_primary", "MFT_hardlink"
) {
    // Only log if name matches our filter
    if !matches_filter(name) {
        return;
    }

    let msg = format!(
        "TreeNode CREATE [{}]: key=({}, {}), name='{}', size={}, mtime={}",
        source, record_number, parent_record_number, name, file_size, modification_time
    );
    info("TREE", &msg);
}

/// Log tree node update - FILTERED by name
pub fn log_tree_node_update(
    record_number: u64,
    parent_record_number: u64,
    name: &str,
    old_size: u64,
    new_size: u64,
    old_mtime: u64,
    new_mtime: u64,
    source: &str,
) {
    // Only log if name matches our filter
    if !matches_filter(name) {
        return;
    }

    let msg = format!(
        "TreeNode UPDATE [{}]: key=({}, {}), name='{}', size: {} -> {}, mtime: {} -> {}",
        source, record_number, parent_record_number, name,
        old_size, new_size, old_mtime, new_mtime
    );
    info("TREE", &msg);
}

/// Log metadata propagation to hard links - FILTERED by name
pub fn log_metadata_propagation(
    record_number: u64,
    source_parent: u64,
    target_parent: u64,
    target_name: &str,
    file_size: u64,
    modification_time: u64,
) {
    // Only log if name matches our filter
    if !matches_filter(target_name) {
        return;
    }

    let msg = format!(
        "Propagate metadata: record={}, from_parent={} to_parent={}, target_name='{}', size={}, mtime={}",
        record_number, source_parent, target_parent, target_name, file_size, modification_time
    );
    info("TREE", &msg);
}

/// Log search result - ALWAYS logs (we want all search results)
pub fn log_search_result(
    index: usize,
    record_number: u64,
    parent_record_number: u64,
    name: &str,
    path: &str,
    file_size: u64,
    modification_time: u64,
    file_reference_number: u64,
) {
    let msg = format!(
        "Search Result #{}: key=({}, {}), frn=0x{:016X}, name='{}', path='{}', size={}, mtime={}",
        index, record_number, parent_record_number, file_reference_number, name, path, file_size, modification_time
    );
    info("SEARCH", &msg);
}

/// Log when a file matches the debug filter
pub fn log_filtered_match(module: &str, name: &str, details: &str) {
    let msg = format!("*** FILTERED MATCH: '{}' - {}", name, details);
    info(module, &msg);
}

/// Log attribute list parsing - FILTERED (requires record_number tracking)
pub fn log_attribute_list(
    record_number: u64,
    attr_type: u32,
    attr_name: &str,
    ext_record: u64,
    starting_vcn: u64,
) {
    // This is low-level, always log for now since we can't filter by name here
    let msg = format!(
        "AttrList entry: base_record={}, type=0x{:02X} ({}), ext_record={}, start_vcn={}",
        record_number, attr_type, attr_name, ext_record, starting_vcn
    );
    debug("MFT", &msg);
}

/// Log extension record resolution
pub fn log_extension_resolution(
    base_record: u64,
    ext_record: u64,
    attr_type: &str,
    found_value: &str,
) {
    let msg = format!(
        "Extension resolution: base={}, ext={}, looking_for={}, found={}",
        base_record, ext_record, attr_type, found_value
    );
    debug("MFT", &msg);
}

/// Log when all hardlinks for a record have been found - FILTERED
pub fn log_all_hardlinks_for_record(record_number: u64, keys: &[(u64, u64, String)]) {
    // Check if any of the names match our filter
    let any_match = keys.iter().any(|(_, _, name)| matches_filter(name));
    if !any_match {
        return;
    }

    let links = keys
        .iter()
        .map(|(rec, parent, name)| format!("(rec={}, parent={}, name='{}')", rec, parent, name))
        .collect::<Vec<_>>()
        .join(", ");

    let msg = format!(
        "All hardlinks for record {}: [{}]",
        record_number, links
    );
    info("TREE", &msg);
}

/// Flush the log file
pub fn flush() {
    if let Some(logger) = LOGGER.get() {
        if let Ok(mut l) = logger.lock() {
            if let Some(ref mut file) = l.file {
                let _ = file.flush();
            }
        }
    }
}

/// Write a separator line for readability
pub fn separator(label: &str) {
    let msg = format!("========== {} ==========", label);
    info("---", &msg);
}

/// Set a filter pattern for targeted debugging (not used with const pattern)
pub fn set_filter(_pattern: Option<String>) {
    // No-op with const pattern
}
