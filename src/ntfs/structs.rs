//! NTFS on-disk structures and constants

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

// ============================================================================
// NTFS Constants
// ============================================================================

/// MFT record signature "FILE"
pub const MFT_RECORD_SIGNATURE: u32 = 0x454C4946; // "FILE" in little-endian

/// Bad MFT record signature "BAAD"
pub const MFT_RECORD_BAD_SIGNATURE: u32 = 0x44414142; // "BAAD"

/// End of attributes marker
pub const ATTRIBUTE_END_MARKER: u32 = 0xFFFFFFFF;

/// Standard MFT record size
pub const DEFAULT_MFT_RECORD_SIZE: u32 = 1024;

/// Standard sector size
pub const SECTOR_SIZE: u32 = 512;

// MFT Record Flags
pub const MFT_RECORD_IN_USE: u16 = 0x0001;
pub const MFT_RECORD_IS_DIRECTORY: u16 = 0x0002;
pub const MFT_RECORD_IN_EXTEND: u16 = 0x0004;
pub const MFT_RECORD_IS_VIEW_INDEX: u16 = 0x0008;

// ============================================================================
// Attribute Types
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum AttributeType {
    StandardInformation = 0x10,
    AttributeList = 0x20,
    FileName = 0x30,
    ObjectId = 0x40,
    SecurityDescriptor = 0x50,
    VolumeName = 0x60,
    VolumeInformation = 0x70,
    Data = 0x80,
    IndexRoot = 0x90,
    IndexAllocation = 0xA0,
    Bitmap = 0xB0,
    ReparsePoint = 0xC0,
    EaInformation = 0xD0,
    Ea = 0xE0,
    LoggedUtilityStream = 0x100,
    End = 0xFFFFFFFF,
}

impl AttributeType {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0x10 => Some(Self::StandardInformation),
            0x20 => Some(Self::AttributeList),
            0x30 => Some(Self::FileName),
            0x40 => Some(Self::ObjectId),
            0x50 => Some(Self::SecurityDescriptor),
            0x60 => Some(Self::VolumeName),
            0x70 => Some(Self::VolumeInformation),
            0x80 => Some(Self::Data),
            0x90 => Some(Self::IndexRoot),
            0xA0 => Some(Self::IndexAllocation),
            0xB0 => Some(Self::Bitmap),
            0xC0 => Some(Self::ReparsePoint),
            0xD0 => Some(Self::EaInformation),
            0xE0 => Some(Self::Ea),
            0x100 => Some(Self::LoggedUtilityStream),
            0xFFFFFFFF => Some(Self::End),
            _ => None,
        }
    }
}

// ============================================================================
// Filename Namespace
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FilenameNamespace {
    Posix = 0,
    Win32 = 1,
    Dos = 2,
    Win32AndDos = 3,
}

impl FilenameNamespace {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Posix),
            1 => Some(Self::Win32),
            2 => Some(Self::Dos),
            3 => Some(Self::Win32AndDos),
            _ => None,
        }
    }

    /// Should this namespace be used for display?
    pub fn is_displayable(&self) -> bool {
        !matches!(self, Self::Dos)
    }
}

// ============================================================================
// NTFS Volume Data (from FSCTL_GET_NTFS_VOLUME_DATA)
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct NtfsVolumeData {
    pub volume_serial_number: u64,
    pub number_sectors: u64,
    pub total_clusters: u64,
    pub free_clusters: u64,
    pub total_reserved: u64,
    pub bytes_per_sector: u32,
    pub bytes_per_cluster: u32,
    pub bytes_per_file_record_segment: u32,
    pub clusters_per_file_record_segment: u32,
    pub mft_valid_data_length: u64,
    pub mft_start_lcn: u64,
    pub mft2_start_lcn: u64,
    pub mft_zone_start: u64,
    pub mft_zone_end: u64,
}

impl NtfsVolumeData {
    /// Parse from raw buffer (0x60 bytes from FSCTL_GET_NTFS_VOLUME_DATA)
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 0x60 {
            return None;
        }

        let mut cursor = Cursor::new(data);

        Some(Self {
            volume_serial_number: cursor.read_u64::<LittleEndian>().ok()?,
            number_sectors: cursor.read_u64::<LittleEndian>().ok()?,
            total_clusters: cursor.read_u64::<LittleEndian>().ok()?,
            free_clusters: cursor.read_u64::<LittleEndian>().ok()?,
            total_reserved: cursor.read_u64::<LittleEndian>().ok()?,
            bytes_per_sector: cursor.read_u32::<LittleEndian>().ok()?,
            bytes_per_cluster: cursor.read_u32::<LittleEndian>().ok()?,
            bytes_per_file_record_segment: cursor.read_u32::<LittleEndian>().ok()?,
            clusters_per_file_record_segment: cursor.read_u32::<LittleEndian>().ok()?,
            mft_valid_data_length: cursor.read_u64::<LittleEndian>().ok()?,
            mft_start_lcn: cursor.read_u64::<LittleEndian>().ok()?,
            mft2_start_lcn: cursor.read_u64::<LittleEndian>().ok()?,
            mft_zone_start: cursor.read_u64::<LittleEndian>().ok()?,
            mft_zone_end: cursor.read_u64::<LittleEndian>().ok()?,
        })
    }

    /// Calculate the byte offset of the MFT on disk
    pub fn mft_byte_offset(&self) -> u64 {
        self.mft_start_lcn * self.bytes_per_cluster as u64
    }

    /// Estimate total MFT records
    pub fn estimated_mft_records(&self) -> u64 {
        self.mft_valid_data_length / self.bytes_per_file_record_segment as u64
    }
}

// ============================================================================
// NTFS Boot Sector
// ============================================================================

/// Parsed NTFS boot sector (first 512 bytes of an NTFS partition)
#[derive(Debug, Clone)]
pub struct NtfsBootSector {
    /// OEM ID - must be "NTFS    " (8 bytes at offset 0x03)
    pub oem_id: [u8; 8],
    /// Bytes per sector (offset 0x0B, typically 512)
    pub bytes_per_sector: u16,
    /// Sectors per cluster (offset 0x0D, typically 8 -> 4096 byte clusters)
    pub sectors_per_cluster: u8,
    /// Total sectors on volume (offset 0x28)
    pub total_sectors: u64,
    /// MFT starting cluster number / LCN (offset 0x30)
    pub mft_cluster_number: u64,
    /// MFT mirror starting cluster number (offset 0x38)
    pub mft_mirror_cluster_number: u64,
    /// Clusters per MFT record (offset 0x40, signed)
    /// If negative, record size = 2^|value| bytes
    /// If positive, record size = value * bytes_per_cluster
    pub clusters_per_mft_record: i8,
    /// Clusters per index block (offset 0x44, signed, same encoding)
    pub clusters_per_index_block: i8,
    /// Volume serial number (offset 0x48)
    pub volume_serial_number: u64,
}

impl NtfsBootSector {
    /// Parse from raw 512-byte boot sector data
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 0x50 {
            return None;
        }

        let mut oem_id = [0u8; 8];
        oem_id.copy_from_slice(&data[0x03..0x0B]);

        let bytes_per_sector = u16::from_le_bytes([data[0x0B], data[0x0C]]);
        let sectors_per_cluster = data[0x0D];
        let total_sectors = u64::from_le_bytes([
            data[0x28], data[0x29], data[0x2A], data[0x2B],
            data[0x2C], data[0x2D], data[0x2E], data[0x2F],
        ]);
        let mft_cluster_number = u64::from_le_bytes([
            data[0x30], data[0x31], data[0x32], data[0x33],
            data[0x34], data[0x35], data[0x36], data[0x37],
        ]);
        let mft_mirror_cluster_number = u64::from_le_bytes([
            data[0x38], data[0x39], data[0x3A], data[0x3B],
            data[0x3C], data[0x3D], data[0x3E], data[0x3F],
        ]);
        let clusters_per_mft_record = data[0x40] as i8;
        let clusters_per_index_block = data[0x44] as i8;
        let volume_serial_number = u64::from_le_bytes([
            data[0x48], data[0x49], data[0x4A], data[0x4B],
            data[0x4C], data[0x4D], data[0x4E], data[0x4F],
        ]);

        Some(Self {
            oem_id,
            bytes_per_sector,
            sectors_per_cluster,
            total_sectors,
            mft_cluster_number,
            mft_mirror_cluster_number,
            clusters_per_mft_record,
            clusters_per_index_block,
            volume_serial_number,
        })
    }

    /// Validate this is an NTFS boot sector
    pub fn is_valid_ntfs(&self) -> bool {
        &self.oem_id == b"NTFS    "
            && self.bytes_per_sector >= 256
            && self.bytes_per_sector.is_power_of_two()
            && self.sectors_per_cluster > 0
            && self.sectors_per_cluster.is_power_of_two()
    }

    /// Calculate bytes per cluster
    pub fn bytes_per_cluster(&self) -> u32 {
        self.bytes_per_sector as u32 * self.sectors_per_cluster as u32
    }

    /// Calculate bytes per MFT record
    /// If clusters_per_mft_record is negative, size = 2^|value|
    /// If positive, size = value * bytes_per_cluster
    pub fn bytes_per_mft_record(&self) -> u32 {
        if self.clusters_per_mft_record < 0 {
            1u32 << (-self.clusters_per_mft_record as u32)
        } else {
            self.clusters_per_mft_record as u32 * self.bytes_per_cluster()
        }
    }

    /// Convert to NtfsVolumeData for compatibility with existing MftParser code.
    /// Note: mft_valid_data_length will be 0 — it must be filled in later
    /// by parsing record 0's $DATA attribute.
    pub fn to_volume_data(&self) -> NtfsVolumeData {
        let bpc = self.bytes_per_cluster();
        let bpr = self.bytes_per_mft_record();
        NtfsVolumeData {
            volume_serial_number: self.volume_serial_number,
            number_sectors: self.total_sectors,
            total_clusters: self.total_sectors / self.sectors_per_cluster as u64,
            free_clusters: 0, // Not available from boot sector
            total_reserved: 0,
            bytes_per_sector: self.bytes_per_sector as u32,
            bytes_per_cluster: bpc,
            bytes_per_file_record_segment: bpr,
            clusters_per_file_record_segment: if bpr >= bpc { bpr / bpc } else { 0 },
            mft_valid_data_length: 0, // Must be filled from record 0's $DATA
            mft_start_lcn: self.mft_cluster_number,
            mft2_start_lcn: self.mft_mirror_cluster_number,
            mft_zone_start: 0, // Not available from boot sector
            mft_zone_end: 0,
        }
    }
}

// ============================================================================
// MFT Record Header
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct MftRecordHeader {
    pub signature: u32,
    pub update_sequence_offset: u16,
    pub update_sequence_size: u16,
    pub log_sequence_number: u64,
    pub sequence_number: u16,
    pub hard_link_count: u16,
    pub first_attribute_offset: u16,
    pub flags: u16,
    pub used_size: u32,
    pub allocated_size: u32,
    pub base_record_reference: u64,
    pub next_attribute_id: u16,
}

impl MftRecordHeader {
    /// Parse MFT record header from raw bytes
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 48 {
            return None;
        }

        let mut cursor = Cursor::new(data);

        Some(Self {
            signature: cursor.read_u32::<LittleEndian>().ok()?,
            update_sequence_offset: cursor.read_u16::<LittleEndian>().ok()?,
            update_sequence_size: cursor.read_u16::<LittleEndian>().ok()?,
            log_sequence_number: cursor.read_u64::<LittleEndian>().ok()?,
            sequence_number: cursor.read_u16::<LittleEndian>().ok()?,
            hard_link_count: cursor.read_u16::<LittleEndian>().ok()?,
            first_attribute_offset: cursor.read_u16::<LittleEndian>().ok()?,
            flags: cursor.read_u16::<LittleEndian>().ok()?,
            used_size: cursor.read_u32::<LittleEndian>().ok()?,
            allocated_size: cursor.read_u32::<LittleEndian>().ok()?,
            base_record_reference: cursor.read_u64::<LittleEndian>().ok()?,
            next_attribute_id: cursor.read_u16::<LittleEndian>().ok()?,
        })
    }

    /// Check if this is a valid MFT record
    pub fn is_valid(&self) -> bool {
        self.signature == MFT_RECORD_SIGNATURE
    }

    /// Check if this record is in use
    pub fn is_in_use(&self) -> bool {
        (self.flags & MFT_RECORD_IN_USE) != 0
    }

    /// Check if this record represents a directory
    pub fn is_directory(&self) -> bool {
        (self.flags & MFT_RECORD_IS_DIRECTORY) != 0
    }

    /// Get the base record number (lower 48 bits)
    pub fn base_record_number(&self) -> u64 {
        self.base_record_reference & 0x0000_FFFF_FFFF_FFFF
    }

    /// Check if this is a base record (not an extension)
    pub fn is_base_record(&self) -> bool {
        self.base_record_reference == 0
    }
}

// ============================================================================
// Attribute Header
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct AttributeHeader {
    pub attribute_type: u32,
    pub length: u32,
    pub non_resident: bool,
    pub name_length: u8,
    pub name_offset: u16,
    pub flags: u16,
    pub attribute_id: u16,
}

#[derive(Debug, Clone)]
pub struct ResidentAttributeHeader {
    pub base: AttributeHeader,
    pub value_length: u32,
    pub value_offset: u16,
    pub indexed_flag: u8,
}

#[derive(Debug, Clone)]
pub struct NonResidentAttributeHeader {
    pub base: AttributeHeader,
    pub lowest_vcn: u64,
    pub highest_vcn: u64,
    pub data_runs_offset: u16,
    pub compression_unit: u16,
    pub allocated_size: u64,
    pub data_size: u64,
    pub initialized_size: u64,
    pub compressed_size: Option<u64>,
}

impl AttributeHeader {
    /// Parse attribute header from raw bytes
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 16 {
            return None;
        }

        let mut cursor = Cursor::new(data);

        Some(Self {
            attribute_type: cursor.read_u32::<LittleEndian>().ok()?,
            length: cursor.read_u32::<LittleEndian>().ok()?,
            non_resident: cursor.read_u8().ok()? != 0,
            name_length: cursor.read_u8().ok()?,
            name_offset: cursor.read_u16::<LittleEndian>().ok()?,
            flags: cursor.read_u16::<LittleEndian>().ok()?,
            attribute_id: cursor.read_u16::<LittleEndian>().ok()?,
        })
    }
}

impl ResidentAttributeHeader {
    /// Parse resident attribute header
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let base = AttributeHeader::from_bytes(data)?;
        if base.non_resident || data.len() < 24 {
            return None;
        }

        let mut cursor = Cursor::new(&data[16..]);

        Some(Self {
            base,
            value_length: cursor.read_u32::<LittleEndian>().ok()?,
            value_offset: cursor.read_u16::<LittleEndian>().ok()?,
            indexed_flag: cursor.read_u8().ok()?,
        })
    }
}

impl NonResidentAttributeHeader {
    /// Parse non-resident attribute header
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        let base = AttributeHeader::from_bytes(data)?;
        if !base.non_resident || data.len() < 64 {
            return None;
        }

        let mut cursor = Cursor::new(&data[16..]);

        let lowest_vcn = cursor.read_u64::<LittleEndian>().ok()?;
        let highest_vcn = cursor.read_u64::<LittleEndian>().ok()?;
        let data_runs_offset = cursor.read_u16::<LittleEndian>().ok()?;
        let compression_unit = cursor.read_u16::<LittleEndian>().ok()?;
        let _padding = cursor.read_u32::<LittleEndian>().ok()?;
        let allocated_size = cursor.read_u64::<LittleEndian>().ok()?;
        let data_size = cursor.read_u64::<LittleEndian>().ok()?;
        let initialized_size = cursor.read_u64::<LittleEndian>().ok()?;

        let compressed_size = if compression_unit > 0 && data.len() >= 72 {
            Some(cursor.read_u64::<LittleEndian>().ok()?)
        } else {
            None
        };

        Some(Self {
            base,
            lowest_vcn,
            highest_vcn,
            data_runs_offset,
            compression_unit,
            allocated_size,
            data_size,
            initialized_size,
            compressed_size,
        })
    }
}

// ============================================================================
// Standard Information Attribute
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct StandardInformation {
    pub creation_time: u64,
    pub modification_time: u64,
    pub mft_modification_time: u64,
    pub access_time: u64,
    pub file_attributes: u32,
    pub max_versions: u32,
    pub version_number: u32,
    pub class_id: u32,
    pub owner_id: u32,
    pub security_id: u32,
    pub quota_charged: u64,
    pub usn: u64,
}

impl StandardInformation {
    /// Parse from resident attribute content
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 48 {
            return None;
        }

        let mut cursor = Cursor::new(data);

        let mut info = Self {
            creation_time: cursor.read_u64::<LittleEndian>().ok()?,
            modification_time: cursor.read_u64::<LittleEndian>().ok()?,
            mft_modification_time: cursor.read_u64::<LittleEndian>().ok()?,
            access_time: cursor.read_u64::<LittleEndian>().ok()?,
            file_attributes: cursor.read_u32::<LittleEndian>().ok()?,
            max_versions: cursor.read_u32::<LittleEndian>().ok()?,
            version_number: cursor.read_u32::<LittleEndian>().ok()?,
            class_id: cursor.read_u32::<LittleEndian>().ok()?,
            ..Default::default()
        };

        // Extended attributes (NTFS 3.0+)
        if data.len() >= 72 {
            info.owner_id = cursor.read_u32::<LittleEndian>().ok()?;
            info.security_id = cursor.read_u32::<LittleEndian>().ok()?;
            info.quota_charged = cursor.read_u64::<LittleEndian>().ok()?;
            info.usn = cursor.read_u64::<LittleEndian>().ok()?;
        }

        Some(info)
    }
}

// ============================================================================
// File Name Attribute
// ============================================================================

#[derive(Debug, Clone)]
pub struct FileNameAttribute {
    pub parent_reference: u64,
    pub creation_time: u64,
    pub modification_time: u64,
    pub mft_modification_time: u64,
    pub access_time: u64,
    pub allocated_size: u64,
    pub data_size: u64,
    pub file_attributes: u32,
    pub reparse_value: u32,
    pub name_length: u8,
    pub namespace: FilenameNamespace,
    pub name: String,
}

impl FileNameAttribute {
    /// Parse from resident attribute content
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 66 {
            return None;
        }

        let mut cursor = Cursor::new(data);

        let parent_reference = cursor.read_u64::<LittleEndian>().ok()?;
        let creation_time = cursor.read_u64::<LittleEndian>().ok()?;
        let modification_time = cursor.read_u64::<LittleEndian>().ok()?;
        let mft_modification_time = cursor.read_u64::<LittleEndian>().ok()?;
        let access_time = cursor.read_u64::<LittleEndian>().ok()?;
        let allocated_size = cursor.read_u64::<LittleEndian>().ok()?;
        let data_size = cursor.read_u64::<LittleEndian>().ok()?;
        let file_attributes = cursor.read_u32::<LittleEndian>().ok()?;
        let reparse_value = cursor.read_u32::<LittleEndian>().ok()?;
        let name_length = cursor.read_u8().ok()?;
        let namespace_byte = cursor.read_u8().ok()?;
        let namespace = FilenameNamespace::from_u8(namespace_byte)?;

        // Read filename (UTF-16LE)
        let name_bytes = name_length as usize * 2;
        if data.len() < 66 + name_bytes {
            return None;
        }

        let name_data = &data[66..66 + name_bytes];
        let name_u16: Vec<u16> = name_data
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        let name = String::from_utf16_lossy(&name_u16);

        Some(Self {
            parent_reference,
            creation_time,
            modification_time,
            mft_modification_time,
            access_time,
            allocated_size,
            data_size,
            file_attributes,
            reparse_value,
            name_length,
            namespace,
            name,
        })
    }

    /// Get the parent record number (lower 48 bits)
    pub fn parent_record_number(&self) -> u64 {
        self.parent_reference & 0x0000_FFFF_FFFF_FFFF
    }
}

// ============================================================================
// Data Run (for non-resident attributes)
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct DataRun {
    pub cluster_count: u64,
    pub lcn_offset: i64, // Signed: can be negative for backwards jumps
    pub is_sparse: bool,
}

impl DataRun {
    /// Decode data runs from raw bytes
    /// Returns list of runs and total cluster count
    pub fn decode_runs(data: &[u8]) -> (Vec<DataRun>, u64) {
        let mut runs = Vec::new();
        let mut total_clusters = 0u64;
        let mut pos = 0;
        let mut current_lcn: i64 = 0;

        while pos < data.len() {
            let header = data[pos];
            if header == 0 {
                break; // End marker
            }

            let length_bytes = (header & 0x0F) as usize;
            let offset_bytes = ((header >> 4) & 0x0F) as usize;

            // Validate
            if length_bytes == 0 || length_bytes > 8 || offset_bytes > 8 {
                break;
            }

            pos += 1;

            // Read cluster count (little-endian, variable length)
            if pos + length_bytes > data.len() {
                break;
            }
            let mut cluster_count = 0u64;
            for i in 0..length_bytes {
                cluster_count |= (data[pos + i] as u64) << (i * 8);
            }
            pos += length_bytes;

            // Read LCN offset (signed, little-endian, variable length)
            let is_sparse = offset_bytes == 0;
            if !is_sparse {
                if pos + offset_bytes > data.len() {
                    break;
                }

                let mut lcn_delta = 0i64;
                for i in 0..offset_bytes {
                    lcn_delta |= (data[pos + i] as i64) << (i * 8);
                }

                // Sign extend if high bit is set
                if offset_bytes < 8 && (data[pos + offset_bytes - 1] & 0x80) != 0 {
                    for i in offset_bytes..8 {
                        lcn_delta |= 0xFFi64 << (i * 8);
                    }
                }

                current_lcn += lcn_delta;
                pos += offset_bytes;
            }

            total_clusters += cluster_count;

            runs.push(DataRun {
                cluster_count,
                lcn_offset: if is_sparse { 0 } else { current_lcn },
                is_sparse,
            });
        }

        (runs, total_clusters)
    }
}

// ============================================================================
// USN Journal Structures
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct UsnJournalData {
    pub usn_journal_id: u64,
    pub first_usn: u64,
    pub next_usn: u64,
    pub lowest_valid_usn: u64,
    pub max_usn: u64,
    pub maximum_size: u64,
    pub allocation_delta: u64,
}

impl UsnJournalData {
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 0x38 {
            return None;
        }

        let mut cursor = Cursor::new(data);

        Some(Self {
            usn_journal_id: cursor.read_u64::<LittleEndian>().ok()?,
            first_usn: cursor.read_u64::<LittleEndian>().ok()?,
            next_usn: cursor.read_u64::<LittleEndian>().ok()?,
            lowest_valid_usn: cursor.read_u64::<LittleEndian>().ok()?,
            max_usn: cursor.read_u64::<LittleEndian>().ok()?,
            maximum_size: cursor.read_u64::<LittleEndian>().ok()?,
            allocation_delta: cursor.read_u64::<LittleEndian>().ok()?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct UsnRecord {
    pub record_length: u32,
    pub major_version: u16,
    pub minor_version: u16,
    pub file_reference_number: u64,
    pub parent_file_reference_number: u64,
    pub usn: u64,
    pub timestamp: u64,
    pub reason: u32,
    pub source_info: u32,
    pub security_id: u32,
    pub file_attributes: u32,
    pub file_name_length: u16,
    pub file_name_offset: u16,
    pub file_name: String,
}

impl UsnRecord {
    /// Parse a USN record (V2 or V3)
    /// V2 record minimum size: 60 bytes (header) + filename
    /// V3 record minimum size: 76 bytes (header) + filename
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 8 {
            return None;
        }

        let record_length = u32::from_le_bytes(data[0..4].try_into().ok()?);
        let major_version = u16::from_le_bytes(data[4..6].try_into().ok()?);
        let minor_version = u16::from_le_bytes(data[6..8].try_into().ok()?);

        // Determine minimum size based on version
        let min_size = if major_version >= 3 { 76 } else { 60 };
        if data.len() < min_size {
            return None;
        }

        // V2 vs V3 have different layouts
        let (file_ref, parent_ref, usn, timestamp, reason, source_info, security_id,
             file_attributes, file_name_length, file_name_offset) = if major_version >= 3 {
            // V3 layout: 128-bit file references
            // Offset 8: FileReferenceNumber (16 bytes)
            // Offset 24: ParentFileReferenceNumber (16 bytes)
            // Offset 40: Usn (8 bytes)
            // Offset 48: TimeStamp (8 bytes)
            // Offset 56: Reason (4 bytes)
            // Offset 60: SourceInfo (4 bytes)
            // Offset 64: SecurityId (4 bytes)
            // Offset 68: FileAttributes (4 bytes)
            // Offset 72: FileNameLength (2 bytes)
            // Offset 74: FileNameOffset (2 bytes)
            // Offset 76: FileName (variable)
            let file_ref = u64::from_le_bytes(data[8..16].try_into().ok()?);
            let parent_ref = u64::from_le_bytes(data[24..32].try_into().ok()?);
            let usn = u64::from_le_bytes(data[40..48].try_into().ok()?);
            let timestamp = u64::from_le_bytes(data[48..56].try_into().ok()?);
            let reason = u32::from_le_bytes(data[56..60].try_into().ok()?);
            let source_info = u32::from_le_bytes(data[60..64].try_into().ok()?);
            let security_id = u32::from_le_bytes(data[64..68].try_into().ok()?);
            let file_attributes = u32::from_le_bytes(data[68..72].try_into().ok()?);
            let file_name_length = u16::from_le_bytes(data[72..74].try_into().ok()?);
            let file_name_offset = u16::from_le_bytes(data[74..76].try_into().ok()?);
            (file_ref, parent_ref, usn, timestamp, reason, source_info, security_id,
             file_attributes, file_name_length, file_name_offset)
        } else {
            // V2 layout: 64-bit file references
            // Offset 8: FileReferenceNumber (8 bytes)
            // Offset 16: ParentFileReferenceNumber (8 bytes)
            // Offset 24: Usn (8 bytes)
            // Offset 32: TimeStamp (8 bytes)
            // Offset 40: Reason (4 bytes)
            // Offset 44: SourceInfo (4 bytes)
            // Offset 48: SecurityId (4 bytes)
            // Offset 52: FileAttributes (4 bytes)
            // Offset 56: FileNameLength (2 bytes)
            // Offset 58: FileNameOffset (2 bytes)
            // Offset 60: FileName (variable)
            let file_ref = u64::from_le_bytes(data[8..16].try_into().ok()?);
            let parent_ref = u64::from_le_bytes(data[16..24].try_into().ok()?);
            let usn = u64::from_le_bytes(data[24..32].try_into().ok()?);
            let timestamp = u64::from_le_bytes(data[32..40].try_into().ok()?);
            let reason = u32::from_le_bytes(data[40..44].try_into().ok()?);
            let source_info = u32::from_le_bytes(data[44..48].try_into().ok()?);
            let security_id = u32::from_le_bytes(data[48..52].try_into().ok()?);
            let file_attributes = u32::from_le_bytes(data[52..56].try_into().ok()?);
            let file_name_length = u16::from_le_bytes(data[56..58].try_into().ok()?);
            let file_name_offset = u16::from_le_bytes(data[58..60].try_into().ok()?);
            (file_ref, parent_ref, usn, timestamp, reason, source_info, security_id,
             file_attributes, file_name_length, file_name_offset)
        };

        // Read filename
        let name_start = file_name_offset as usize;
        let name_end = name_start + file_name_length as usize;
        if name_end > data.len() || name_end > record_length as usize {
            return None;
        }

        let name_data = &data[name_start..name_end];
        let name_u16: Vec<u16> = name_data
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        let file_name = String::from_utf16_lossy(&name_u16);

        Some(Self {
            record_length,
            major_version,
            minor_version,
            file_reference_number: file_ref,
            parent_file_reference_number: parent_ref,
            usn,
            timestamp,
            reason,
            source_info,
            security_id,
            file_attributes,
            file_name_length,
            file_name_offset,
            file_name,
        })
    }

    /// Get file reference number (lower 48 bits)
    pub fn file_record_number(&self) -> u64 {
        self.file_reference_number & 0x0000_FFFF_FFFF_FFFF
    }

    /// Get parent reference number (lower 48 bits)
    pub fn parent_record_number(&self) -> u64 {
        self.parent_file_reference_number & 0x0000_FFFF_FFFF_FFFF
    }
}

// USN Reason flags
pub mod usn_reason {
    pub const DATA_OVERWRITE: u32 = 0x00000001;
    pub const DATA_EXTEND: u32 = 0x00000002;
    pub const DATA_TRUNCATION: u32 = 0x00000004;
    pub const NAMED_DATA_OVERWRITE: u32 = 0x00000010;
    pub const NAMED_DATA_EXTEND: u32 = 0x00000020;
    pub const NAMED_DATA_TRUNCATION: u32 = 0x00000040;
    pub const FILE_CREATE: u32 = 0x00000100;
    pub const FILE_DELETE: u32 = 0x00000200;
    pub const EA_CHANGE: u32 = 0x00000400;
    pub const SECURITY_CHANGE: u32 = 0x00000800;
    pub const RENAME_OLD_NAME: u32 = 0x00001000;
    pub const RENAME_NEW_NAME: u32 = 0x00002000;
    pub const INDEXABLE_CHANGE: u32 = 0x00004000;
    pub const BASIC_INFO_CHANGE: u32 = 0x00008000;
    pub const HARD_LINK_CHANGE: u32 = 0x00010000;
    pub const COMPRESSION_CHANGE: u32 = 0x00020000;
    pub const ENCRYPTION_CHANGE: u32 = 0x00040000;
    pub const OBJECT_ID_CHANGE: u32 = 0x00080000;
    pub const REPARSE_POINT_CHANGE: u32 = 0x00100000;
    pub const STREAM_CHANGE: u32 = 0x00200000;
    pub const CLOSE: u32 = 0x80000000;
}

// ============================================================================
// Attribute List Entry (for files with attributes in extension records)
// ============================================================================

/// Entry in an $ATTRIBUTE_LIST attribute
/// Used when a file has too many attributes to fit in a single MFT record
#[derive(Debug, Clone)]
pub struct AttributeListEntry {
    /// Attribute type (e.g., 0x30 for FILE_NAME, 0x80 for DATA)
    pub attribute_type: u32,
    /// Length of this entry
    pub entry_length: u16,
    /// Length of attribute name
    pub name_length: u8,
    /// Offset to attribute name
    pub name_offset: u8,
    /// Starting VCN (for non-resident attributes)
    pub starting_vcn: u64,
    /// MFT file reference where attribute is stored
    pub mft_reference: u64,
    /// Attribute ID
    pub attribute_id: u16,
    /// Attribute name (if any)
    pub name: Option<String>,
}

impl AttributeListEntry {
    /// Parse an attribute list entry from raw bytes
    /// Returns the entry and the number of bytes consumed
    pub fn from_bytes(data: &[u8]) -> Option<(Self, usize)> {
        if data.len() < 26 {
            return None;
        }

        let attribute_type = u32::from_le_bytes(data[0..4].try_into().ok()?);
        let entry_length = u16::from_le_bytes(data[4..6].try_into().ok()?);
        let name_length = data[6];
        let name_offset = data[7];
        let starting_vcn = u64::from_le_bytes(data[8..16].try_into().ok()?);
        let mft_reference = u64::from_le_bytes(data[16..24].try_into().ok()?);
        let attribute_id = u16::from_le_bytes(data[24..26].try_into().ok()?);

        // Sanity check
        if entry_length < 26 || entry_length as usize > data.len() {
            return None;
        }

        // Parse attribute name if present
        let name = if name_length > 0 {
            let name_start = name_offset as usize;
            let name_bytes = name_length as usize * 2;
            if name_start + name_bytes <= data.len() {
                let name_data = &data[name_start..name_start + name_bytes];
                let name_u16: Vec<u16> = name_data
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                Some(String::from_utf16_lossy(&name_u16))
            } else {
                None
            }
        } else {
            None
        };

        Some((Self {
            attribute_type,
            entry_length,
            name_length,
            name_offset,
            starting_vcn,
            mft_reference,
            attribute_id,
            name,
        }, entry_length as usize))
    }

    /// Get the MFT record number (lower 48 bits)
    pub fn record_number(&self) -> u64 {
        self.mft_reference & 0x0000_FFFF_FFFF_FFFF
    }

    /// Check if this entry points to an extension record (not the base record)
    pub fn is_extension(&self, base_record_number: u64) -> bool {
        self.record_number() != base_record_number
    }
}

/// Parse all entries from an Attribute List
pub fn parse_attribute_list(data: &[u8]) -> Vec<AttributeListEntry> {
    let mut entries = Vec::new();
    let mut offset = 0;

    while offset + 26 <= data.len() {
        match AttributeListEntry::from_bytes(&data[offset..]) {
            Some((entry, consumed)) => {
                if consumed == 0 {
                    break;
                }
                entries.push(entry);
                offset += consumed;
            }
            None => break,
        }
    }

    entries
}

// ============================================================================
// FILETIME conversion utilities
// ============================================================================

/// Convert Windows FILETIME (100-nanosecond intervals since 1601) to Unix timestamp
pub fn filetime_to_unix(filetime: u64) -> i64 {
    // Difference between 1601 and 1970 in 100-nanosecond intervals
    const EPOCH_DIFF: u64 = 116444736000000000;

    if filetime < EPOCH_DIFF {
        return 0;
    }

    ((filetime - EPOCH_DIFF) / 10_000_000) as i64
}

/// Convert Windows FILETIME to chrono DateTime
pub fn filetime_to_datetime(filetime: u64) -> chrono::DateTime<chrono::Utc> {
    use chrono::{TimeZone, Utc};
    let unix_ts = filetime_to_unix(filetime);
    Utc.timestamp_opt(unix_ts, 0).single().unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap())
}

// ============================================================================
// File Attributes
// ============================================================================

pub mod file_attributes {
    pub const READONLY: u32 = 0x00000001;
    pub const HIDDEN: u32 = 0x00000002;
    pub const SYSTEM: u32 = 0x00000004;
    pub const DIRECTORY: u32 = 0x00000010;
    pub const ARCHIVE: u32 = 0x00000020;
    pub const DEVICE: u32 = 0x00000040;
    pub const NORMAL: u32 = 0x00000080;
    pub const TEMPORARY: u32 = 0x00000100;
    pub const SPARSE_FILE: u32 = 0x00000200;
    pub const REPARSE_POINT: u32 = 0x00000400;
    pub const COMPRESSED: u32 = 0x00000800;
    pub const OFFLINE: u32 = 0x00001000;
    pub const NOT_CONTENT_INDEXED: u32 = 0x00002000;
    pub const ENCRYPTED: u32 = 0x00004000;
}
