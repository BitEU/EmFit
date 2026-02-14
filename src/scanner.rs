//! Main Scanner Module
//!
//! Orchestrates the scanning process, combining USN enumeration
//! with MFT parsing for complete and accurate results.
//! Supports direct physical drive reading (bypasses NTFS driver) for maximum reliability.

use crate::error::{Result, EmFitError};
use crate::file_tree::{FileTree, TreeBuilder, TreeNode};
use crate::logging;
use crate::ntfs::{
    open_volume, FileEntry, MftParser, MftRecordFetcher, NtfsVolumeData,
    UsnEntry, UsnMonitor, UsnScanner, VolumeIO, open_physical_drive_for_volume,
};
use crate::ntfs::winapi::get_ntfs_volume_data;
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ============================================================================
// Scanner Configuration
// ============================================================================

/// Configuration for the scanner
#[derive(Debug, Clone)]
pub struct ScanConfig {
    /// Scan using USN Journal (fast) if available
    pub use_usn: bool,
    /// Scan using direct MFT reading
    pub use_mft: bool,
    /// Use direct physical drive access (bypasses NTFS driver)
    pub use_physical_drive: bool,
    /// Include hidden files
    pub include_hidden: bool,
    /// Include system files
    pub include_system: bool,
    /// Calculate directory sizes
    pub calculate_sizes: bool,
    /// Show progress during scan
    pub show_progress: bool,
    /// Number of MFT records to read per batch
    pub batch_size: usize,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            use_usn: false,
            use_mft: true,
            use_physical_drive: true,
            include_hidden: true,
            include_system: true,
            calculate_sizes: true,
            show_progress: true,
            batch_size: 1024,
        }
    }
}

// ============================================================================
// Scan Progress
// ============================================================================

/// Progress information during scan
#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub phase: ScanPhase,
    pub records_processed: u64,
    pub records_total: u64,
    pub files_found: u64,
    pub directories_found: u64,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPhase {
    Initializing,
    UsnEnumeration,
    MftReading,
    BuildingTree,
    CalculatingSizes,
    Complete,
}

impl ScanPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScanPhase::Initializing => "Initializing",
            ScanPhase::UsnEnumeration => "USN Enumeration",
            ScanPhase::MftReading => "MFT Reading",
            ScanPhase::BuildingTree => "Building Tree",
            ScanPhase::CalculatingSizes => "Calculating Sizes",
            ScanPhase::Complete => "Complete",
        }
    }
}

// ============================================================================
// Volume Scanner
// ============================================================================

/// Main scanner for a single volume
pub struct VolumeScanner {
    /// Drive letter
    drive_letter: char,
    /// Configuration
    config: ScanConfig,
    /// Volume data
    volume_data: Option<NtfsVolumeData>,
    /// Cancellation flag
    cancelled: Arc<AtomicBool>,
}

impl VolumeScanner {
    /// Create a new scanner for a drive
    pub fn new(drive_letter: char) -> Self {
        Self {
            drive_letter: drive_letter.to_ascii_uppercase(),
            config: ScanConfig::default(),
            volume_data: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Configure the scanner
    pub fn with_config(mut self, config: ScanConfig) -> Self {
        self.config = config;
        self
    }

    /// Get cancellation token
    pub fn cancel_token(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }

    /// Cancel the scan
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Check if cancelled
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Perform the scan
    pub fn scan(&mut self) -> Result<FileTree> {
        let start_time = Instant::now();

        logging::separator(&format!("SCAN START: Drive {}", self.drive_letter));
        logging::info("SCANNER", &format!("Config: usn={}, mft={}, physical={}, hidden={}, system={}",
            self.config.use_usn, self.config.use_mft, self.config.use_physical_drive,
            self.config.include_hidden, self.config.include_system));

        // Initialize progress bar
        let pb = if self.config.show_progress {
            let pb = ProgressBar::new(100);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")
                    .unwrap()
                    .progress_chars("#>-"),
            );
            Some(pb)
        } else {
            None
        };

        // Phase 1: Open I/O source — try physical drive first, fall back to volume
        if let Some(ref pb) = pb {
            if self.config.use_physical_drive {
                pb.set_message("Opening physical drive...");
            } else {
                pb.set_message("Opening volume...");
            }
        }

        let (io, is_physical) = if self.config.use_physical_drive {
            match open_physical_drive_for_volume(self.drive_letter) {
                Ok(io) => {
                    logging::info("SCANNER", "Physical drive mode active)");
                    if let Some(ref pb) = pb {
                        pb.set_message("Physical drive mode active");
                    }
                    (io, true)
                }
                Err(e) => {
                    logging::warn("SCANNER", &format!(
                        "Physical drive access failed: {}. Falling back to volume mode.", e
                    ));
                    if let Some(ref pb) = pb {
                        pb.set_message("Falling back to volume mode...");
                    }
                    let handle = open_volume(self.drive_letter)?;
                    let volume_data = get_ntfs_volume_data(&handle)?;
                    (VolumeIO::Volume { handle, volume_data }, false)
                }
            }
        } else {
            let handle = open_volume(self.drive_letter)?;
            let volume_data = get_ntfs_volume_data(&handle)?;
            (VolumeIO::Volume { handle, volume_data }, false)
        };

        let volume_data = io.volume_data().clone();
        self.volume_data = Some(volume_data.clone());

        // Create MFT parser with the I/O source
        let mut parser = MftParser::new(io)?;
        parser.load_mft_extents(self.drive_letter)?;

        // Update volume_data after extents are loaded (mft_valid_data_length may have been set)
        let volume_data = parser.volume_data().clone();
        self.volume_data = Some(volume_data.clone());

        let estimated_records = parser.estimated_records();
        if let Some(ref pb) = pb {
            pb.set_length(estimated_records);
            pb.set_message(format!(
                "Volume: {} ({} estimated records, {})",
                self.drive_letter, estimated_records,
                if is_physical { "physical drive" } else { "volume handle" }
            ));
        }

        // Create TreeBuilder with volume info
        let mut builder = TreeBuilder::with_volume_info(
            self.drive_letter,
            volume_data.bytes_per_file_record_segment,
        );

        // Set up MftRecordFetcher for on-demand parent resolution
        match MftRecordFetcher::new(
            self.drive_letter,
            volume_data.clone(),
            parser.mft_extents(),
            is_physical,
        ) {
            Ok(fetcher) => {
                builder.set_record_fetcher(Arc::new(fetcher));
            }
            Err(e) => {
                logging::warn("SCANNER", &format!("Failed to create MftRecordFetcher: {}", e));
            }
        }

        // Phase 2: Try USN enumeration first (fast path) — only in volume mode
        let mut usn_success = false;

        if self.config.use_usn && !is_physical {
            if let Some(ref pb) = pb {
                pb.set_message("Scanning via USN Journal...");
            }

            match self.scan_via_usn(&mut builder, pb.as_ref()) {
                Ok(count) => {
                    usn_success = true;
                    logging::info("SCANNER", &format!("USN phase complete: {} entries", count));
                    if let Some(ref pb) = pb {
                        pb.set_message(format!("USN: {} entries found", count));
                    }
                }
                Err(e) => {
                    logging::warn("SCANNER", &format!("USN unavailable: {}", e));
                    if let Some(ref pb) = pb {
                        pb.set_message(format!("USN unavailable: {}", e));
                    }
                }
            }
        }

        if self.is_cancelled() {
            return Err(EmFitError::Cancelled);
        }

        // Phase 3: MFT reading for size information (or full scan if USN failed)
        if self.config.use_mft && (self.config.calculate_sizes || !usn_success) {
            logging::separator("MFT SCAN PHASE");
            if let Some(ref pb) = pb {
                if usn_success {
                    pb.set_message("Reading MFT for file sizes...");
                } else if is_physical {
                    pb.set_message("Scanning via MFT (physical drive)...");
                } else {
                    pb.set_message("Scanning via MFT...");
                }
                pb.set_position(0);
            }

            self.scan_via_mft_with_parser(&mut parser, &mut builder, pb.as_ref())?;
            logging::info("SCANNER", "MFT phase complete");
        }

        if self.is_cancelled() {
            return Err(EmFitError::Cancelled);
        }

        // Phase 4: Build and finalize tree
        logging::separator("BUILD TREE PHASE");
        if let Some(ref pb) = pb {
            pb.set_message("Building file tree...");
        }

        let tree = builder.build();

        logging::info("SCANNER", &format!(
            "Scan complete: {} files, {} dirs, {:.2}s ({})",
            tree.stats.total_files, tree.stats.total_directories,
            start_time.elapsed().as_secs_f64(),
            if is_physical { "physical drive" } else { "volume" }
        ));

        if let Some(ref pb) = pb {
            pb.finish_with_message(format!(
                "Complete: {} files, {} directories ({:.2}s)",
                tree.stats.total_files,
                tree.stats.total_directories,
                start_time.elapsed().as_secs_f64()
            ));
        }

        logging::flush();
        Ok(tree)
    }

    /// Scan using USN Journal
    fn scan_via_usn(
        &self,
        builder: &mut TreeBuilder,
        pb: Option<&ProgressBar>,
    ) -> Result<u64> {
        let handle = open_volume(self.drive_letter)?;
        let mut usn_scanner = UsnScanner::new(handle);
        usn_scanner.initialize()?;

        let count = AtomicU64::new(0);
        let mut entries = Vec::new();

        usn_scanner.enumerate_all(|entry| {
            // Filter if needed
            if !self.config.include_hidden
                && (entry.attributes & crate::ntfs::structs::file_attributes::HIDDEN) != 0
            {
                return;
            }
            if !self.config.include_system
                && (entry.attributes & crate::ntfs::structs::file_attributes::SYSTEM) != 0
            {
                return;
            }

            entries.push(entry);
            let c = count.fetch_add(1, Ordering::Relaxed);

            if let Some(pb) = pb {
                if c % 10000 == 0 {
                    pb.set_position(c);
                    pb.set_message(format!("USN: {} entries", c));
                }
            }
        })?;

        builder.add_usn_entries(entries.into_iter());

        Ok(count.load(Ordering::Relaxed))
    }

    /// Scan using direct MFT reading with a pre-created parser
    fn scan_via_mft_with_parser(
        &self,
        parser: &mut MftParser,
        builder: &mut TreeBuilder,
        pb: Option<&ProgressBar>,
    ) -> Result<()> {
        let total_records = parser.estimated_records();
        let batch_size = self.config.batch_size;
        let mut processed = 0u64;
        let mut all_entries = Vec::new();

        while processed < total_records {
            if self.is_cancelled() {
                return Err(EmFitError::Cancelled);
            }

            let batch_count = std::cmp::min(batch_size, (total_records - processed) as usize);

            match parser.read_records_batch(processed, batch_count) {
                Ok(batch) => {
                    let batch_entries = parser.parse_batch_with_extensions(batch);

                    for entry in batch_entries {
                        if !self.config.include_hidden && entry.is_hidden() {
                            continue;
                        }
                        if !self.config.include_system && entry.is_system() {
                            continue;
                        }

                        all_entries.push(entry);
                    }
                }
                Err(e) => {
                    if !e.is_recoverable() {
                        break;
                    }
                }
            }

            processed += batch_count as u64;

            if let Some(pb) = pb {
                pb.set_position(processed);
                if processed % 50000 == 0 {
                    pb.set_message(format!(
                        "MFT: {}/{} records ({} files)",
                        processed,
                        total_records,
                        all_entries.len()
                    ));
                }
            }
        }

        builder.add_file_entries(all_entries.into_iter());
        Ok(())
    }

    /// Get volume data after scan
    pub fn volume_data(&self) -> Option<&NtfsVolumeData> {
        self.volume_data.as_ref()
    }
}

// ============================================================================
// Multi-Volume Scanner
// ============================================================================

/// Scan multiple volumes in parallel
pub struct MultiVolumeScanner {
    config: ScanConfig,
}

impl MultiVolumeScanner {
    pub fn new() -> Self {
        Self {
            config: ScanConfig::default(),
        }
    }

    pub fn with_config(mut self, config: ScanConfig) -> Self {
        self.config = config;
        self
    }

    /// Scan multiple drives
    pub fn scan_drives(&self, drive_letters: &[char]) -> Vec<(char, Result<FileTree>)> {
        // Could use rayon for parallel scanning
        drive_letters
            .iter()
            .map(|&letter| {
                let mut scanner = VolumeScanner::new(letter).with_config(self.config.clone());
                let result = scanner.scan();
                (letter, result)
            })
            .collect()
    }

    /// Detect all NTFS volumes
    pub fn detect_ntfs_volumes() -> Vec<char> {
        let mut volumes = Vec::new();

        for letter in 'A'..='Z' {
            if let Ok(handle) = open_volume(letter) {
                if get_ntfs_volume_data(&handle).is_ok() {
                    volumes.push(letter);
                }
            }
        }

        volumes
    }
}

impl Default for MultiVolumeScanner {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Real-time Monitor Integration
// ============================================================================

/// Wrapper for monitoring file system changes
pub struct ChangeMonitor {
    drive_letter: char,
    monitor: Option<UsnMonitor>,
}

impl ChangeMonitor {
    /// Create a new change monitor for a drive
    pub fn new(drive_letter: char) -> Result<Self> {
        let handle = open_volume(drive_letter)?;
        let mut scanner = UsnScanner::new(handle);
        scanner.initialize()?;

        let journal = scanner
            .journal_data()
            .ok_or_else(|| EmFitError::UsnJournalNotActive(drive_letter.to_string()))?;

        // Need to reopen handle for monitor (scanner took ownership)
        let handle = open_volume(drive_letter)?;
        let monitor = UsnMonitor::new(handle, journal.usn_journal_id, journal.next_usn as i64);

        Ok(Self {
            drive_letter,
            monitor: Some(monitor),
        })
    }

    /// Poll for changes and apply to tree
    pub fn apply_changes(&mut self, tree: &mut FileTree) -> Result<usize> {
        let monitor = self
            .monitor
            .as_mut()
            .ok_or_else(|| EmFitError::UsnJournalNotActive(self.drive_letter.to_string()))?;

        let changes = monitor.poll_changes()?;
        let count = changes.len();

        for change in changes {
            match change.reason {
                crate::ntfs::ChangeReason::Created => {
                    let node = TreeNode {
                        record_number: change.record_number,
                        parent_record_number: change.parent_record_number,
                        name: change.name,
                        attributes: change.attributes,
                        is_directory: (change.attributes
                            & crate::ntfs::structs::file_attributes::DIRECTORY)
                            != 0,
                        ..Default::default()
                    };
                    tree.insert(node);
                }
                crate::ntfs::ChangeReason::Deleted => {
                    // Mark as deleted or remove
                    // (Implementation depends on desired behavior)
                }
                crate::ntfs::ChangeReason::RenamedTo => {
                    // Update name
                    // (Would need mutable access pattern)
                }
                _ => {
                    // Handle other changes
                }
            }
        }

        Ok(count)
    }
}
