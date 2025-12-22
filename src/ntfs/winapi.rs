//! Windows API bindings for NTFS operations
//!
//! Safe wrappers around Win32 APIs for volume access and IOCTL operations.

use crate::error::{Result, EmFitError};
use crate::ntfs::structs::*;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;

// ============================================================================
// IOCTL Control Codes (from winioctl.h and our reverse engineering)
// ============================================================================

// Standard NTFS IOCTLs
pub const FSCTL_GET_NTFS_VOLUME_DATA: u32 = 0x00090064;
pub const FSCTL_GET_NTFS_FILE_RECORD: u32 = 0x00090068;
pub const FSCTL_GET_VOLUME_BITMAP: u32 = 0x0009006F;
pub const FSCTL_GET_RETRIEVAL_POINTERS: u32 = 0x00090073;
pub const FSCTL_ENUM_USN_DATA: u32 = 0x000900B3;
pub const FSCTL_READ_USN_JOURNAL: u32 = 0x000900BB;
pub const FSCTL_QUERY_USN_JOURNAL: u32 = 0x000900F4;
pub const FSCTL_DELETE_USN_JOURNAL: u32 = 0x000900F8;
pub const FSCTL_CREATE_USN_JOURNAL: u32 = 0x000900E7;

// Disk geometry
pub const IOCTL_DISK_GET_DRIVE_GEOMETRY: u32 = 0x00070000;

// File attributes for CreateFile
pub const GENERIC_READ: u32 = 0x80000000;
pub const GENERIC_WRITE: u32 = 0x40000000;
pub const FILE_SHARE_READ: u32 = 0x00000001;
pub const FILE_SHARE_WRITE: u32 = 0x00000002;
pub const FILE_SHARE_DELETE: u32 = 0x00000004;
pub const OPEN_EXISTING: u32 = 3;
pub const FILE_FLAG_NO_BUFFERING: u32 = 0x20000000;
pub const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x02000000;

pub const INVALID_HANDLE_VALUE: isize = -1;

// ============================================================================
// Safe Handle Wrapper
// ============================================================================

/// RAII wrapper for Windows HANDLE
pub struct SafeHandle {
    handle: isize,
}

impl SafeHandle {
    /// Create from raw handle
    pub fn new(handle: isize) -> Option<Self> {
        if handle == INVALID_HANDLE_VALUE || handle == 0 {
            None
        } else {
            Some(Self { handle })
        }
    }

    /// Get raw handle value
    pub fn as_raw(&self) -> isize {
        self.handle
    }

    /// Check if handle is valid
    pub fn is_valid(&self) -> bool {
        self.handle != INVALID_HANDLE_VALUE && self.handle != 0
    }
}

impl Drop for SafeHandle {
    fn drop(&mut self) {
        if self.is_valid() {
            unsafe {
                windows::Win32::Foundation::CloseHandle(
                    windows::Win32::Foundation::HANDLE(self.handle as *mut std::ffi::c_void)
                );
            }
        }
    }
}

// ============================================================================
// Volume Operations
// ============================================================================

/// Open a volume for raw read access
pub fn open_volume(drive_letter: char) -> Result<SafeHandle> {
    let path = format!("\\\\.\\{}:", drive_letter);
    open_volume_path(&path)
}

/// Open a volume by path
pub fn open_volume_path(path: &str) -> Result<SafeHandle> {
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE,
    };
    use windows::Win32::Foundation::HANDLE;
    use windows::core::PCWSTR;

    let wide_path: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe {
        CreateFileW(
            PCWSTR::from_raw(wide_path.as_ptr()),
            GENERIC_READ,
            FILE_SHARE_MODE(FILE_SHARE_READ | FILE_SHARE_WRITE),
            None,
            windows::Win32::Storage::FileSystem::OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(FILE_FLAG_NO_BUFFERING),
            HANDLE::default(),
        )
    };

    match handle {
        Ok(h) => SafeHandle::new(h.0 as isize)
            .ok_or_else(|| EmFitError::VolumeOpenError(path.to_string(), std::io::Error::last_os_error())),
        Err(e) => Err(EmFitError::VolumeOpenError(path.to_string(), std::io::Error::from_raw_os_error(e.code().0 as i32))),
    }
}

/// Open a file by path with read access
pub fn open_file_read(path: &str) -> Result<SafeHandle> {
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE,
    };
    use windows::Win32::Foundation::HANDLE;
    use windows::core::PCWSTR;

    let wide_path: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe {
        CreateFileW(
            PCWSTR::from_raw(wide_path.as_ptr()),
            GENERIC_READ,
            FILE_SHARE_MODE(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE),
            None,
            windows::Win32::Storage::FileSystem::OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(FILE_FLAG_BACKUP_SEMANTICS),
            HANDLE::default(),
        )
    };

    match handle {
        Ok(h) => SafeHandle::new(h.0 as isize)
            .ok_or_else(|| EmFitError::IoError(std::io::Error::last_os_error())),
        Err(e) => Err(EmFitError::IoError(std::io::Error::from_raw_os_error(e.code().0 as i32))),
    }
}

// ============================================================================
// IOCTL Operations
// ============================================================================

/// Send a DeviceIoControl request
pub fn device_io_control(
    handle: &SafeHandle,
    control_code: u32,
    in_buffer: Option<&[u8]>,
    out_buffer: &mut [u8],
) -> Result<u32> {
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::Foundation::HANDLE;

    let mut bytes_returned: u32 = 0;

    let (in_ptr, in_size) = match in_buffer {
        Some(buf) => (buf.as_ptr() as *const std::ffi::c_void, buf.len() as u32),
        None => (ptr::null(), 0),
    };

    let result = unsafe {
        DeviceIoControl(
            HANDLE(handle.as_raw() as *mut std::ffi::c_void),
            control_code,
            Some(in_ptr),
            in_size,
            Some(out_buffer.as_mut_ptr() as *mut std::ffi::c_void),
            out_buffer.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    if result.is_ok() {
        Ok(bytes_returned)
    } else {
        let error = std::io::Error::last_os_error();
        Err(EmFitError::WindowsError(format!(
            "DeviceIoControl(0x{:08X}) failed: {}",
            control_code, error
        )))
    }
}

/// Get NTFS volume data
pub fn get_ntfs_volume_data(handle: &SafeHandle) -> Result<NtfsVolumeData> {
    let mut buffer = [0u8; 0x60];
    device_io_control(handle, FSCTL_GET_NTFS_VOLUME_DATA, None, &mut buffer)?;

    NtfsVolumeData::from_bytes(&buffer).ok_or_else(|| {
        EmFitError::VolumeDataError("Failed to parse NTFS volume data".to_string())
    })
}

/// Fetch a single MFT record using FSCTL_GET_NTFS_FILE_RECORD
///
/// This allows fetching any MFT record by number, even if it wasn't enumerated
/// by the USN journal. Used for resolving missing parent directories.
pub fn get_ntfs_file_record(
    handle: &SafeHandle,
    record_number: u64,
    bytes_per_record: u32,
) -> Result<Vec<u8>> {
    // Input: 8-byte file reference number (record number in lower 48 bits)
    let input = record_number.to_le_bytes();

    // Output buffer: 8 (returned FRN) + 4 (record length) + record data
    let buffer_size = 12 + bytes_per_record as usize;
    let mut buffer = vec![0u8; buffer_size];

    let bytes_returned = device_io_control(
        handle,
        FSCTL_GET_NTFS_FILE_RECORD,
        Some(&input),
        &mut buffer,
    )?;

    if bytes_returned < 12 {
        return Err(EmFitError::MftReadError(format!(
            "Short response for record {}: {} bytes",
            record_number, bytes_returned
        )));
    }

    // The returned FRN tells us what record was actually returned
    // (might be different if the requested record is not in use)
    let returned_frn = u64::from_le_bytes(buffer[0..8].try_into().unwrap());
    let returned_record = returned_frn & 0x0000_FFFF_FFFF_FFFF; // Lower 48 bits

    if returned_record != record_number {
        return Err(EmFitError::MftReadError(format!(
            "Record {} not in use (returned {})",
            record_number, returned_record
        )));
    }

    let record_length = u32::from_le_bytes(buffer[8..12].try_into().unwrap()) as usize;

    if record_length == 0 || 12 + record_length > buffer.len() {
        return Err(EmFitError::MftReadError(format!(
            "Invalid record length {} for record {}",
            record_length, record_number
        )));
    }

    Ok(buffer[12..12 + record_length].to_vec())
}

/// Query USN Journal information
pub fn query_usn_journal(handle: &SafeHandle) -> Result<UsnJournalData> {
    let mut buffer = [0u8; 0x38];

    match device_io_control(handle, FSCTL_QUERY_USN_JOURNAL, None, &mut buffer) {
        Ok(_) => UsnJournalData::from_bytes(&buffer).ok_or_else(|| {
            EmFitError::UsnJournalError("Failed to parse USN journal data".to_string())
        }),
        Err(e) => {
            // Check for "journal not active" error
            let error_str = e.to_string();
            if error_str.contains("1178") || error_str.contains("1179") {
                Err(EmFitError::UsnJournalNotActive("Volume".to_string()))
            } else {
                Err(e)
            }
        }
    }
}

/// Read raw bytes from volume at specified offset
pub fn read_volume_at(handle: &SafeHandle, offset: u64, buffer: &mut [u8]) -> Result<usize> {
    use windows::Win32::Storage::FileSystem::{ReadFile, SetFilePointerEx};
    use windows::Win32::Foundation::HANDLE;

    // Seek to offset
    let mut new_pos: i64 = 0;
    let seek_result = unsafe {
        SetFilePointerEx(
            HANDLE(handle.as_raw() as *mut std::ffi::c_void),
            offset as i64,
            Some(&mut new_pos),
            windows::Win32::Storage::FileSystem::FILE_BEGIN,
        )
    };

    if seek_result.is_err() {
        return Err(EmFitError::IoError(std::io::Error::last_os_error()));
    }

    // Read data
    let mut bytes_read: u32 = 0;
    let read_result = unsafe {
        ReadFile(
            HANDLE(handle.as_raw() as *mut std::ffi::c_void),
            Some(buffer),
            Some(&mut bytes_read),
            None,
        )
    };

    if read_result.is_ok() {
        Ok(bytes_read as usize)
    } else {
        Err(EmFitError::IoError(std::io::Error::last_os_error()))
    }
}

// ============================================================================
// MFT Enumeration via USN
// ============================================================================

/// Input structure for FSCTL_ENUM_USN_DATA (V0 - legacy, for Windows XP/2003)
#[repr(C, packed)]
#[allow(dead_code)]
pub struct MftEnumDataV0 {
    pub start_file_reference_number: u64,
    pub low_usn: i64,
    pub high_usn: i64,
}

/// Input structure for FSCTL_ENUM_USN_DATA (V1 - Windows 8+)
/// This version includes min/max major version fields for filtering USN record versions
#[repr(C, packed)]
pub struct MftEnumDataV1 {
    pub start_file_reference_number: u64,
    pub low_usn: i64,
    pub high_usn: i64,
    pub min_major_version: u16,
    pub max_major_version: u16,
}

/// Enumerate USN data (all files on volume)
/// Uses V1 structure for Windows 8+ compatibility with USN record version 2 and 3
pub fn enum_usn_data(
    handle: &SafeHandle,
    start_frn: u64,
    high_usn: i64,
    buffer: &mut [u8],
) -> Result<(u64, usize)> {
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::Foundation::HANDLE;

    // Use V1 structure to request both V2 and V3 USN records
    let input = MftEnumDataV1 {
        start_file_reference_number: start_frn,
        low_usn: 0,
        high_usn,
        min_major_version: 2,  // Accept V2 records
        max_major_version: 3,  // Accept V3 records (128-bit file IDs)
    };

    let input_bytes = unsafe {
        std::slice::from_raw_parts(
            &input as *const MftEnumDataV1 as *const u8,
            std::mem::size_of::<MftEnumDataV1>(),
        )
    };

    let mut bytes_returned: u32 = 0;

    let result = unsafe {
        DeviceIoControl(
            HANDLE(handle.as_raw() as *mut std::ffi::c_void),
            FSCTL_ENUM_USN_DATA,
            Some(input_bytes.as_ptr() as *const std::ffi::c_void),
            input_bytes.len() as u32,
            Some(buffer.as_mut_ptr() as *mut std::ffi::c_void),
            buffer.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    if result.is_err() {
        let error = std::io::Error::last_os_error();
        let error_code = error.raw_os_error().unwrap_or(0);

        // ERROR_HANDLE_EOF (38) means enumeration is complete - not an error
        if error_code == 38 {
            return Ok((0, 0));
        }

        return Err(EmFitError::WindowsError(format!(
            "DeviceIoControl(0x{:08X}) failed: {}",
            FSCTL_ENUM_USN_DATA, error
        )));
    }

    if bytes_returned < 8 {
        return Ok((0, 0));
    }

    // First 8 bytes are the next file reference number
    let next_frn = u64::from_le_bytes(buffer[0..8].try_into().unwrap());

    Ok((next_frn, bytes_returned as usize))
}

// ============================================================================
// USN Journal Reading
// ============================================================================

/// Input structure for FSCTL_READ_USN_JOURNAL
#[repr(C, packed)]
pub struct ReadUsnJournalData {
    pub start_usn: i64,
    pub reason_mask: u32,
    pub return_only_on_close: u32,
    pub timeout: u64,
    pub bytes_to_wait_for: u64,
    pub usn_journal_id: u64,
}

/// Read USN journal entries
pub fn read_usn_journal(
    handle: &SafeHandle,
    journal_id: u64,
    start_usn: i64,
    reason_mask: u32,
    buffer: &mut [u8],
) -> Result<(i64, usize)> {
    let input = ReadUsnJournalData {
        start_usn,
        reason_mask,
        return_only_on_close: 0,
        timeout: 0,
        bytes_to_wait_for: 0,
        usn_journal_id: journal_id,
    };

    let input_bytes = unsafe {
        std::slice::from_raw_parts(
            &input as *const ReadUsnJournalData as *const u8,
            std::mem::size_of::<ReadUsnJournalData>(),
        )
    };

    let bytes_returned =
        device_io_control(handle, FSCTL_READ_USN_JOURNAL, Some(input_bytes), buffer)?;

    if bytes_returned < 8 {
        return Ok((start_usn, 0));
    }

    // First 8 bytes are the next USN
    let next_usn = i64::from_le_bytes(buffer[0..8].try_into().unwrap());

    Ok((next_usn, bytes_returned as usize))
}

// ============================================================================
// Retrieval Pointers (for fragmented MFT)
// ============================================================================

/// Get retrieval pointers for a file (cluster extents)
#[derive(Debug, Clone)]
pub struct Extent {
    pub vcn: u64,
    pub lcn: u64,
    pub cluster_count: u64,
}

pub fn get_retrieval_pointers(handle: &SafeHandle, start_vcn: u64) -> Result<Vec<Extent>> {
    let mut buffer = vec![0u8; 64 * 1024]; // 64KB buffer

    let input = start_vcn.to_le_bytes();

    let bytes_returned =
        device_io_control(handle, FSCTL_GET_RETRIEVAL_POINTERS, Some(&input), &mut buffer)?;

    if bytes_returned < 16 {
        return Ok(Vec::new());
    }

    // Parse RETRIEVAL_POINTERS_BUFFER
    let extent_count = u32::from_le_bytes(buffer[0..4].try_into().unwrap()) as usize;
    let _starting_vcn = u64::from_le_bytes(buffer[8..16].try_into().unwrap());

    let mut extents = Vec::with_capacity(extent_count);
    let mut pos = 16;
    let mut prev_vcn = start_vcn;

    for _ in 0..extent_count {
        if pos + 16 > bytes_returned as usize {
            break;
        }

        let next_vcn = u64::from_le_bytes(buffer[pos..pos + 8].try_into().unwrap());
        let lcn = u64::from_le_bytes(buffer[pos + 8..pos + 16].try_into().unwrap());

        extents.push(Extent {
            vcn: prev_vcn,
            lcn,
            cluster_count: next_vcn - prev_vcn,
        });

        prev_vcn = next_vcn;
        pos += 16;
    }

    Ok(extents)
}

// ============================================================================
// Error Code Helpers
// ============================================================================

/// Get the last Windows error code
pub fn get_last_error() -> u32 {
    unsafe { windows::Win32::Foundation::GetLastError().0 }
}

/// Check if error indicates "journal not active"
pub fn is_journal_not_active_error(code: u32) -> bool {
    code == 1178 || code == 1179 // ERROR_JOURNAL_NOT_ACTIVE, ERROR_JOURNAL_DELETE_IN_PROGRESS
}
