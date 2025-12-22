//! EmFit - High-performance NTFS file scanner
//!
//! Combines the best of WizTree (direct MFT reading) and Everything (USN Journal)
//! for ultra-fast, accurate file system scanning.
//!
//! # Features
//!
//! - **USN Journal Scanning**: Instant enumeration of all files
//! - **Direct MFT Reading**: Accurate file sizes and attributes
//! - **Real-time Monitoring**: Track file system changes as they happen
//! - **Fast Search**: Instant file search across indexed volumes using trigram index
//! - **Size Analysis**: WizTree-style directory size breakdown
//! - **Persistence**: Save/load index for instant startup
//!
//! # Example
//!
//! ```no_run
//! use emfit::{VolumeScanner, ScanConfig, SearchIndex, parse_query};
//!
//! fn main() -> emfit::Result<()> {
//!     // Scan C: drive
//!     let mut scanner = VolumeScanner::new('C')
//!         .with_config(ScanConfig::default());
//!     
//!     let tree = scanner.scan()?;
//!     
//!     println!("Files: {}", tree.stats.total_files);
//!     println!("Directories: {}", tree.stats.total_directories);
//!     println!("Total size: {} bytes", tree.stats.total_size);
//!     
//!     // Build search index for instant searching
//!     let index = SearchIndex::new('C');
//!     for entry in tree.iter() {
//!         let node = entry.value();
//!         index.add(IndexEntry::from_tree_node(node));
//!     }
//!     
//!     // Search for files
//!     let query = parse_query("*.txt");
//!     let results = index.search(&query, 100);
//!     for hit in results {
//!         println!("{}: {}", hit.entry.name, hit.path.unwrap_or_default());
//!     }
//!     
//!     Ok(())
//! }
//! ```

#![cfg(windows)]

pub mod error;
pub mod file_tree;
pub mod gui;
pub mod ntfs;
pub mod scanner;

// Re-export main types
pub use error::{Result, EmFitError};
pub use file_tree::{FileTree, NodeKey, SearchResult, TreeBuilder, TreeNode, TreeStats};
pub use scanner::{
    ChangeMonitor, MultiVolumeScanner, ScanConfig, ScanPhase, ScanProgress, VolumeScanner,
};

// Re-export NTFS types that users might need
pub use ntfs::{
    ChangeEvent, ChangeReason, FileEntry, NtfsVolumeData, UsnEntry, UsnJournalData,
};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Format bytes as human-readable string
pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
    
    if bytes == 0 {
        return "0 B".to_string();
    }
    
    let exp = (bytes as f64).log(1024.0).floor() as usize;
    let exp = exp.min(UNITS.len() - 1);
    let size = bytes as f64 / 1024_f64.powi(exp as i32);
    
    if exp == 0 {
        format!("{} B", bytes)
    } else {
        format!("{:.2} {}", size, UNITS[exp])
    }
}

/// Format a Windows FILETIME as a human-readable date string
pub fn format_filetime(filetime: u64) -> String {
    use ntfs::structs::filetime_to_datetime;
    filetime_to_datetime(filetime).format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Application configuration
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Automatically load index on startup
    pub auto_load_index: bool,
    /// Automatically save index on exit
    pub auto_save_index: bool,
    /// Enable real-time monitoring
    pub enable_monitoring: bool,
    /// Drives to scan/monitor
    pub drives: Vec<char>,
    /// Maximum search results
    pub max_search_results: usize,
    /// Include hidden files
    pub include_hidden: bool,
    /// Include system files
    pub include_system: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            auto_load_index: true,
            auto_save_index: true,
            enable_monitoring: true,
            drives: vec!['C'],
            max_search_results: 1000,
            include_hidden: true,
            include_system: true,
        }
    }
}
