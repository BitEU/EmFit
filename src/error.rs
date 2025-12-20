//! Error types for RustyScan
//!
//! Comprehensive error handling for all NTFS operations

use thiserror::Error;

/// Main error type for RustyScan operations
#[derive(Error, Debug)]
pub enum RustyScanError {
    #[error("Failed to open volume '{0}': {1}")]
    VolumeOpenError(String, std::io::Error),

    #[error("Volume '{0}' is not an NTFS filesystem")]
    NotNtfsVolume(String),

    #[error("Failed to get NTFS volume data: {0}")]
    VolumeDataError(String),

    #[error("Failed to read MFT: {0}")]
    MftReadError(String),

    #[error("Invalid MFT record at index {0}: {1}")]
    InvalidMftRecord(u64, String),

    #[error("MFT fixup verification failed at record {0}")]
    FixupVerificationFailed(u64),

    #[error("Invalid attribute at offset {0}: {1}")]
    InvalidAttribute(u32, String),

    #[error("USN Journal error: {0}")]
    UsnJournalError(String),

    #[error("USN Journal not active on volume '{0}'")]
    UsnJournalNotActive(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Windows API error: {0}")]
    WindowsError(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Access denied: {0}")]
    AccessDenied(String),

    #[error("Operation cancelled")]
    Cancelled,

    #[error("Buffer too small: needed {needed}, got {got}")]
    BufferTooSmall { needed: usize, got: usize },

    #[error("Data run decode error: {0}")]
    DataRunError(String),

    #[error("Record {0} references non-existent parent {1}")]
    OrphanedRecord(u64, u64),
}

/// Result type alias for RustyScan operations
pub type Result<T> = std::result::Result<T, RustyScanError>;

impl RustyScanError {
    /// Create a Windows API error from a raw error code
    pub fn from_win32(code: u32, context: &str) -> Self {
        RustyScanError::WindowsError(format!("{}: Win32 error code {}", context, code))
    }

    /// Check if this error is recoverable (scan can continue)
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            RustyScanError::InvalidMftRecord(_, _)
                | RustyScanError::FixupVerificationFailed(_)
                | RustyScanError::InvalidAttribute(_, _)
                | RustyScanError::OrphanedRecord(_, _)
        )
    }
}
