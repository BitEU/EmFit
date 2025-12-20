//! NTFS filesystem support
//!
//! This module provides comprehensive NTFS parsing capabilities:
//! - Direct MFT (Master File Table) reading and parsing
//! - USN (Update Sequence Number) Journal scanning
//! - Real-time change monitoring
//! - Fixup verification for data integrity

pub mod mft;
pub mod structs;
pub mod usn;
pub mod winapi;

// Re-export commonly used types
pub use mft::{FileEntry, MftParser};
pub use structs::{
    AttributeType, DataRun, FileNameAttribute, FilenameNamespace, MftRecordHeader,
    NtfsVolumeData, StandardInformation, UsnJournalData, UsnRecord,
};
pub use usn::{ChangeEvent, ChangeReason, HybridScanner, UsnEntry, UsnMonitor, UsnScanner};
pub use winapi::{open_volume, SafeHandle};
