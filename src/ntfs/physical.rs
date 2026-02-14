//! Physical Drive Access
//!
//! Provides direct physical drive reading.
//! Bypasses the NTFS filesystem driver by reading raw sectors from \\.\PhysicalDriveN.

use crate::error::{EmFitError, Result};
use crate::logging;
use crate::ntfs::mft::{extract_parent_info, apply_fixup_standalone};
use crate::ntfs::structs::*;
use crate::ntfs::winapi::*;
use std::sync::Mutex;

// ============================================================================
// VolumeIO — Abstraction over volume reads
// ============================================================================

/// Abstraction over volume I/O source.
/// Either a volume handle (\\.\C:) or a physical drive handle (\\.\PhysicalDrive0)
/// with a partition offset applied to all reads.
pub enum VolumeIO {
    /// Traditional volume handle — reads are volume-relative
    Volume {
        handle: SafeHandle,
        volume_data: NtfsVolumeData,
    },
    /// Physical drive handle — reads are offset by partition_offset
    Physical {
        handle: SafeHandle,
        partition_offset: u64,
        volume_data: NtfsVolumeData,
    },
}

impl VolumeIO {
    /// Read bytes at a volume-relative offset.
    /// For Physical mode, the partition_offset is added automatically.
    pub fn read_at(&self, volume_offset: u64, buffer: &mut [u8]) -> Result<usize> {
        match self {
            VolumeIO::Volume { handle, .. } => {
                read_volume_at(handle, volume_offset, buffer)
            }
            VolumeIO::Physical { handle, partition_offset, .. } => {
                let physical_offset = partition_offset + volume_offset;
                read_volume_at(handle, physical_offset, buffer)
            }
        }
    }

    /// Get the volume data
    pub fn volume_data(&self) -> &NtfsVolumeData {
        match self {
            VolumeIO::Volume { volume_data, .. } => volume_data,
            VolumeIO::Physical { volume_data, .. } => volume_data,
        }
    }

    /// Get a mutable reference to the volume data (for updating mft_valid_data_length)
    pub fn volume_data_mut(&mut self) -> &mut NtfsVolumeData {
        match self {
            VolumeIO::Volume { volume_data, .. } => volume_data,
            VolumeIO::Physical { volume_data, .. } => volume_data,
        }
    }

    /// Check if this is physical drive mode
    pub fn is_physical(&self) -> bool {
        matches!(self, VolumeIO::Physical { .. })
    }
}

// ============================================================================
// PhysicalDriveReader — Opens and configures physical drive access
// ============================================================================

/// Opens a physical drive for a given drive letter by:
/// 1. Mapping the drive letter to a physical disk number + partition offset
/// 2. Opening \\.\PhysicalDriveN
/// 3. Reading and parsing the NTFS boot sector
/// 4. Producing a VolumeIO::Physical
pub fn open_physical_drive_for_volume(drive_letter: char) -> Result<VolumeIO> {
    logging::info("PHYSICAL", &format!("Opening physical drive for {}:", drive_letter));

    // Step 1: Map drive letter to physical disk
    let vol_handle = open_volume(drive_letter)?;
    let extent = get_volume_disk_extents(&vol_handle)?;
    drop(vol_handle); // Close the volume handle — we don't need it anymore

    logging::info("PHYSICAL", &format!(
        "Volume {}:  ->  PhysicalDrive{} at offset {} ({:.2} GB), length {} ({:.2} GB)",
        drive_letter,
        extent.disk_number,
        extent.starting_offset,
        extent.starting_offset as f64 / (1024.0 * 1024.0 * 1024.0),
        extent.extent_length,
        extent.extent_length as f64 / (1024.0 * 1024.0 * 1024.0),
    ));

    // Step 2: Open the physical drive
    let phys_handle = open_physical_drive(extent.disk_number)?;

    // Step 3: Read and parse NTFS boot sector at partition offset
    let mut boot_buffer = vec![0u8; 512];
    let bytes_read = read_volume_at(&phys_handle, extent.starting_offset, &mut boot_buffer)?;
    if bytes_read < 512 {
        return Err(EmFitError::PhysicalDriveError(format!(
            "Short read for boot sector: {} bytes", bytes_read
        )));
    }

    let boot_sector = NtfsBootSector::from_bytes(&boot_buffer)
        .ok_or(EmFitError::NotNtfsBootSector)?;

    if !boot_sector.is_valid_ntfs() {
        return Err(EmFitError::NotNtfsBootSector);
    }

    logging::info("PHYSICAL", &format!(
        "NTFS boot sector: bytes_per_sector={}, sectors_per_cluster={}, bytes_per_cluster={}, \
         mft_start_lcn={}, bytes_per_mft_record={}, serial={:016X}",
        boot_sector.bytes_per_sector,
        boot_sector.sectors_per_cluster,
        boot_sector.bytes_per_cluster(),
        boot_sector.mft_cluster_number,
        boot_sector.bytes_per_mft_record(),
        boot_sector.volume_serial_number,
    ));

    let volume_data = boot_sector.to_volume_data();

    Ok(VolumeIO::Physical {
        handle: phys_handle,
        partition_offset: extent.starting_offset,
        volume_data,
    })
}

// ============================================================================
// MftRecordFetcher — On-demand parent resolution from MFT
// ============================================================================

/// Fetches individual MFT records for on-demand parent resolution.
/// Replaces FSCTL_GET_NTFS_FILE_RECORD by reading directly from the I/O source.
pub struct MftRecordFetcher {
    /// I/O source (must be behind a Mutex because read_at needs &self but
    /// SetFilePointerEx is not thread-safe on the same handle)
    io: Mutex<FetcherIO>,
    /// Volume data for offset calculations
    volume_data: NtfsVolumeData,
    /// MFT extents for fragmented MFT lookups
    mft_extents: Vec<Extent>,
}

struct FetcherIO {
    handle: SafeHandle,
    /// Only used for Physical mode
    partition_offset: u64,
}

impl FetcherIO {
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let physical_offset = self.partition_offset + offset;
        read_volume_at(&self.handle, physical_offset, buffer)
    }
}

impl MftRecordFetcher {
    /// Create a new MftRecordFetcher.
    /// `drive_letter` is used to open a separate handle for the fetcher.
    /// `volume_data` and `mft_extents` come from the MftParser after it has loaded extents.
    /// `is_physical` determines whether to use physical drive or volume handle mode.
    pub fn new(
        drive_letter: char,
        volume_data: NtfsVolumeData,
        mft_extents: Vec<Extent>,
        is_physical: bool,
    ) -> Result<Self> {
        let (handle, partition_offset) = if is_physical {
            let vol_handle = open_volume(drive_letter)?;
            let extent = get_volume_disk_extents(&vol_handle)?;
            drop(vol_handle);
            let phys_handle = open_physical_drive(extent.disk_number)?;
            (phys_handle, extent.starting_offset)
        } else {
            let handle = open_volume(drive_letter)?;
            (handle, 0u64)
        };

        Ok(Self {
            io: Mutex::new(FetcherIO { handle, partition_offset }),
            volume_data,
            mft_extents,
        })
    }

    /// Fetch parent info (name, parent_record_number) for a given MFT record.
    /// Returns None if the record is invalid or not in use.
    pub fn fetch_parent_info(&self, record_number: u64) -> Option<(String, u64)> {
        let record_size = self.volume_data.bytes_per_file_record_segment as usize;
        let mut buffer = vec![0u8; record_size];

        let offset = self.calculate_record_offset(record_number);

        let io = self.io.lock().ok()?;
        let bytes_read = io.read_at(offset, &mut buffer).ok()?;
        drop(io);

        if bytes_read < record_size {
            return None;
        }

        // Raw disk reads need fixup applied before parsing
        let header = MftRecordHeader::from_bytes(&buffer)?;
        if !header.is_valid() || !header.is_in_use() {
            return None;
        }
        apply_fixup_standalone(&mut buffer, &header).ok()?;

        // Now extract parent info (same function used for FSCTL_GET_NTFS_FILE_RECORD data)
        extract_parent_info(&buffer)
    }

    /// Calculate byte offset of an MFT record (volume-relative)
    fn calculate_record_offset(&self, record_number: u64) -> u64 {
        let record_size = self.volume_data.bytes_per_file_record_segment as u64;

        if self.mft_extents.is_empty() {
            self.volume_data.mft_byte_offset() + record_number * record_size
        } else {
            let target_vcn = record_number * record_size / self.volume_data.bytes_per_cluster as u64;
            let offset_in_cluster =
                (record_number * record_size) % self.volume_data.bytes_per_cluster as u64;

            for extent in &self.mft_extents {
                if target_vcn >= extent.vcn && target_vcn < extent.vcn + extent.cluster_count {
                    let cluster_offset = target_vcn - extent.vcn;
                    let lcn = extent.lcn + cluster_offset;
                    return lcn * self.volume_data.bytes_per_cluster as u64 + offset_in_cluster;
                }
            }

            // Fallback
            self.volume_data.mft_byte_offset() + record_number * record_size
        }
    }
}
