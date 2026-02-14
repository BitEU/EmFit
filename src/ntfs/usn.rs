//! USN Journal Scanner
//!
//! Fast file enumeration using the NTFS USN (Update Sequence Number) Journal.
//! This is the method used by Everything for instant file indexing.

use crate::error::{Result, EmFitError};
use crate::logging;
use crate::ntfs::mft::FileEntry;
use crate::ntfs::structs::*;
use crate::ntfs::winapi::*;
use std::collections::HashMap;

// ============================================================================
// USN Entry (lightweight version of FileEntry for initial scan)
// ============================================================================

/// Lightweight file entry from USN enumeration
#[derive(Debug, Clone)]
pub struct UsnEntry {
    pub record_number: u64,
    pub parent_record_number: u64,
    /// Full file reference number including sequence number (for OpenFileById)
    pub file_reference_number: u64,
    pub name: String,
    pub attributes: u32,
    pub is_directory: bool,
}

impl UsnEntry {
    /// Convert to full FileEntry (without size info - requires MFT lookup)
    pub fn to_file_entry(&self) -> FileEntry {
        FileEntry {
            record_number: self.record_number,
            parent_record_number: self.parent_record_number,
            file_reference_number: self.file_reference_number,
            name: self.name.clone(),
            attributes: self.attributes,
            is_directory: self.is_directory,
            is_valid: true,
            ..Default::default()
        }
    }
}

// ============================================================================
// USN Scanner
// ============================================================================

/// Scanner that uses USN Journal for fast file enumeration
pub struct UsnScanner {
    handle: SafeHandle,
    journal_data: Option<UsnJournalData>,
    buffer: Vec<u8>,
}

impl UsnScanner {
    /// Create a new USN scanner for a volume
    pub fn new(handle: SafeHandle) -> Self {
        Self {
            handle,
            journal_data: None,
            buffer: vec![0u8; 64 * 1024], // 64KB buffer
        }
    }

    /// Initialize by querying the USN journal
    pub fn initialize(&mut self) -> Result<()> {
        self.journal_data = Some(query_usn_journal(&self.handle)?);
        Ok(())
    }

    /// Check if USN journal is available
    pub fn is_available(&self) -> bool {
        self.journal_data.is_some()
    }

    /// Get journal data
    pub fn journal_data(&self) -> Option<&UsnJournalData> {
        self.journal_data.as_ref()
    }

    /// Enumerate all files using FSCTL_ENUM_USN_DATA
    ///
    /// This is the fastest way to enumerate all files on an NTFS volume.
    /// Returns entries via callback to avoid memory pressure.
    pub fn enumerate_all<F>(&mut self, mut callback: F) -> Result<u64>
    where
        F: FnMut(UsnEntry),
    {
        let journal = self
            .journal_data
            .as_ref()
            .ok_or_else(|| EmFitError::UsnJournalError("Journal not initialized".to_string()))?;

        let high_usn = journal.next_usn as i64;
        let mut start_frn: u64 = 0;
        let mut count: u64 = 0;

        loop {
            let (next_frn, bytes_returned) =
                enum_usn_data(&self.handle, start_frn, high_usn, &mut self.buffer)?;

            if bytes_returned <= 8 {
                break;
            }

            // Parse USN records from buffer (skip first 8 bytes which is next FRN)
            let mut offset = 8;
            // Minimum USN record is 60 bytes (V2) or 76 bytes (V3), but we check inside from_bytes
            while offset + 8 < bytes_returned {
                // First read record_length to know how big this record is
                if offset + 4 > bytes_returned {
                    break;
                }
                let record_len = u32::from_le_bytes(
                    self.buffer[offset..offset + 4].try_into().unwrap_or([0; 4])
                ) as usize;

                // Sanity check record length
                if record_len < 60 || record_len > 0x10000 || offset + record_len > bytes_returned {
                    break;
                }

                if let Some(record) = UsnRecord::from_bytes(&self.buffer[offset..offset + record_len]) {
                    let entry = UsnEntry {
                        record_number: record.file_record_number(),
                        parent_record_number: record.parent_record_number(),
                        file_reference_number: record.file_reference_number, // Full FRN with sequence
                        name: record.file_name.clone(),
                        attributes: record.file_attributes,
                        is_directory: (record.file_attributes & file_attributes::DIRECTORY) != 0,
                    };

                    // Log USN entries for debugging (filtered by name pattern if set)
                    logging::log_usn_entry(
                        entry.record_number,
                        entry.parent_record_number,
                        entry.file_reference_number,
                        &entry.name,
                        entry.attributes,
                        entry.is_directory,
                    );

                    callback(entry);
                    count += 1;

                    offset += record_len;
                } else {
                    // Skip this record and try the next
                    offset += record_len;
                }
            }

            if next_frn == 0 || next_frn == start_frn {
                break;
            }
            start_frn = next_frn;
        }

        Ok(count)
    }

    /// Enumerate to a HashMap for quick parent lookups
    pub fn enumerate_to_map(&mut self) -> Result<HashMap<u64, UsnEntry>> {
        let mut map = HashMap::new();

        self.enumerate_all(|entry| {
            map.insert(entry.record_number, entry);
        })?;

        Ok(map)
    }
}

// ============================================================================
// USN Change Monitor
// ============================================================================

/// Monitor for real-time file system changes
pub struct UsnMonitor {
    handle: SafeHandle,
    journal_id: u64,
    last_usn: i64,
    buffer: Vec<u8>,
}

/// Represents a file system change event
#[derive(Debug, Clone)]
pub struct ChangeEvent {
    pub record_number: u64,
    pub parent_record_number: u64,
    pub name: String,
    pub reason: ChangeReason,
    pub attributes: u32,
    pub usn: u64,
    pub timestamp: u64,
}

/// Type of change that occurred
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeReason {
    Created,
    Deleted,
    Modified,
    RenamedFrom,
    RenamedTo,
    AttributeChange,
    SecurityChange,
    Other(u32),
}

impl ChangeReason {
    fn from_usn_reason(reason: u32) -> Self {
        if (reason & usn_reason::FILE_CREATE) != 0 {
            ChangeReason::Created
        } else if (reason & usn_reason::FILE_DELETE) != 0 {
            ChangeReason::Deleted
        } else if (reason & usn_reason::RENAME_OLD_NAME) != 0 {
            ChangeReason::RenamedFrom
        } else if (reason & usn_reason::RENAME_NEW_NAME) != 0 {
            ChangeReason::RenamedTo
        } else if (reason & usn_reason::SECURITY_CHANGE) != 0 {
            ChangeReason::SecurityChange
        } else if (reason & usn_reason::BASIC_INFO_CHANGE) != 0 {
            ChangeReason::AttributeChange
        } else if (reason
            & (usn_reason::DATA_OVERWRITE
                | usn_reason::DATA_EXTEND
                | usn_reason::DATA_TRUNCATION))
            != 0
        {
            ChangeReason::Modified
        } else {
            ChangeReason::Other(reason)
        }
    }

    /// Is this a significant change we should track?
    pub fn is_significant(&self) -> bool {
        !matches!(self, ChangeReason::Other(_))
    }
}

impl UsnMonitor {
    /// Create a new change monitor
    pub fn new(handle: SafeHandle, journal_id: u64, start_usn: i64) -> Self {
        Self {
            handle,
            journal_id,
            last_usn: start_usn,
            buffer: vec![0u8; 64 * 1024],
        }
    }

    /// Poll for new changes
    ///
    /// Returns changes since last poll. Call this periodically to stay up to date.
    pub fn poll_changes(&mut self) -> Result<Vec<ChangeEvent>> {
        let mut changes = Vec::new();

        // Read all changes since last_usn
        let reason_mask = usn_reason::FILE_CREATE
            | usn_reason::FILE_DELETE
            | usn_reason::RENAME_OLD_NAME
            | usn_reason::RENAME_NEW_NAME
            | usn_reason::DATA_OVERWRITE
            | usn_reason::DATA_EXTEND
            | usn_reason::DATA_TRUNCATION
            | usn_reason::BASIC_INFO_CHANGE
            | usn_reason::SECURITY_CHANGE;

        let (next_usn, bytes_returned) = read_usn_journal(
            &self.handle,
            self.journal_id,
            self.last_usn,
            reason_mask,
            &mut self.buffer,
        )?;

        if bytes_returned > 8 {
            let mut offset = 8;
            while offset + 60 < bytes_returned {
                if let Some(record) = UsnRecord::from_bytes(&self.buffer[offset..]) {
                    let record_len = record.record_length as usize;
                    if record_len < 60 || offset + record_len > bytes_returned {
                        break;
                    }

                    let reason = ChangeReason::from_usn_reason(record.reason);

                    // Only include closed changes (complete operations)
                    if (record.reason & usn_reason::CLOSE) != 0 || reason.is_significant() {
                        changes.push(ChangeEvent {
                            record_number: record.file_record_number(),
                            parent_record_number: record.parent_record_number(),
                            name: record.file_name.clone(),
                            reason,
                            attributes: record.file_attributes,
                            usn: record.usn,
                            timestamp: record.timestamp,
                        });
                    }

                    offset += record_len;
                } else {
                    break;
                }
            }
        }

        self.last_usn = next_usn;
        Ok(changes)
    }

    /// Get current USN position
    pub fn current_usn(&self) -> i64 {
        self.last_usn
    }

    /// Reset to a specific USN position
    pub fn seek_to(&mut self, usn: i64) {
        self.last_usn = usn;
    }
}

// ============================================================================
// Combined Scanner (USN + MFT fallback)
// ============================================================================

/// High-level scanner that uses USN when available, falls back to MFT
pub struct HybridScanner {
    usn_scanner: Option<UsnScanner>,
    drive_letter: char,
}

impl HybridScanner {
    /// Create a new hybrid scanner for a drive
    pub fn new(drive_letter: char) -> Result<Self> {
        let handle = open_volume(drive_letter)?;
        let mut usn_scanner = UsnScanner::new(handle);

        // Try to initialize USN journal
        let usn_available = usn_scanner.initialize().is_ok();

        Ok(Self {
            usn_scanner: if usn_available {
                Some(usn_scanner)
            } else {
                None
            },
            drive_letter,
        })
    }

    /// Check if USN scanning is available
    pub fn has_usn(&self) -> bool {
        self.usn_scanner.is_some()
    }

    /// Get the scanning method being used
    pub fn scan_method(&self) -> &'static str {
        if self.usn_scanner.is_some() {
            "USN Journal (fast)"
        } else {
            "Direct MFT (fallback)"
        }
    }

    /// Scan all files, using the best available method
    pub fn scan_all(&mut self) -> Result<HashMap<u64, UsnEntry>> {
        if let Some(ref mut usn) = self.usn_scanner {
            usn.enumerate_to_map()
        } else {
            // Would need MFT parser here - for now return error
            Err(EmFitError::UsnJournalNotActive(
                self.drive_letter.to_string(),
            ))
        }
    }
}
