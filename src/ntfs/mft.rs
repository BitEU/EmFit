//! MFT (Master File Table) Parser
//!
//! Handles reading and parsing MFT records with fixup verification,
//! attribute extraction, and data run decoding.

use crate::error::{Result, RustyScanError};
use crate::ntfs::structs::*;
use crate::ntfs::winapi::*;
use std::collections::HashMap;

// ============================================================================
// Parsed File Entry
// ============================================================================

/// Complete parsed information for a file/directory
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// MFT record number
    pub record_number: u64,
    /// Parent directory record number
    pub parent_record_number: u64,
    /// File name (best available: Win32 > Win32+DOS > POSIX > DOS)
    pub name: String,
    /// File size in bytes
    pub file_size: u64,
    /// Allocated size on disk
    pub allocated_size: u64,
    /// File attributes
    pub attributes: u32,
    /// Is this a directory?
    pub is_directory: bool,
    /// Creation time (FILETIME)
    pub creation_time: u64,
    /// Last modification time (FILETIME)
    pub modification_time: u64,
    /// Last access time (FILETIME)
    pub access_time: u64,
    /// Hard link count
    pub hard_link_count: u16,
    /// Data runs for non-resident files
    pub data_runs: Vec<DataRun>,
    /// Alternate data streams (name -> size)
    pub alternate_streams: HashMap<String, u64>,
    /// Is this record valid/in use?
    pub is_valid: bool,
    /// Has this record been fully parsed?
    pub is_complete: bool,
}

impl Default for FileEntry {
    fn default() -> Self {
        Self {
            record_number: 0,
            parent_record_number: 0,
            name: String::new(),
            file_size: 0,
            allocated_size: 0,
            attributes: 0,
            is_directory: false,
            creation_time: 0,
            modification_time: 0,
            access_time: 0,
            hard_link_count: 0,
            data_runs: Vec::new(),
            alternate_streams: HashMap::new(),
            is_valid: false,
            is_complete: false,
        }
    }
}

impl FileEntry {
    /// Check if file is hidden
    pub fn is_hidden(&self) -> bool {
        (self.attributes & file_attributes::HIDDEN) != 0
    }

    /// Check if file is system
    pub fn is_system(&self) -> bool {
        (self.attributes & file_attributes::SYSTEM) != 0
    }

    /// Check if file is compressed
    pub fn is_compressed(&self) -> bool {
        (self.attributes & file_attributes::COMPRESSED) != 0
    }

    /// Check if file is sparse
    pub fn is_sparse(&self) -> bool {
        (self.attributes & file_attributes::SPARSE_FILE) != 0
    }

    /// Check if this is a reparse point (symlink, junction, etc)
    pub fn is_reparse_point(&self) -> bool {
        (self.attributes & file_attributes::REPARSE_POINT) != 0
    }
}

// ============================================================================
// MFT Parser
// ============================================================================

/// MFT Parser handles reading and decoding MFT records
pub struct MftParser {
    /// Volume handle
    handle: SafeHandle,
    /// NTFS volume data
    volume_data: NtfsVolumeData,
    /// MFT extents (for fragmented MFT)
    mft_extents: Vec<Extent>,
    /// Sector-aligned read buffer
    read_buffer: Vec<u8>,
}

impl MftParser {
    /// Create a new MFT parser for a volume
    pub fn new(handle: SafeHandle, volume_data: NtfsVolumeData) -> Result<Self> {
        // Allocate aligned buffer for reading
        let buffer_size = (volume_data.bytes_per_file_record_segment * 16) as usize;
        let read_buffer = vec![0u8; buffer_size];

        Ok(Self {
            handle,
            volume_data,
            mft_extents: Vec::new(),
            read_buffer,
        })
    }

    /// Get MFT extents by opening $MFT directly
    pub fn load_mft_extents(&mut self, drive_letter: char) -> Result<()> {
        let mft_path = format!("{}:\\$MFT", drive_letter);

        match open_file_read(&mft_path) {
            Ok(mft_handle) => {
                self.mft_extents = get_retrieval_pointers(&mft_handle, 0)?;
                Ok(())
            }
            Err(_) => {
                // If we can't open $MFT directly, we'll read sequentially
                self.mft_extents.clear();
                Ok(())
            }
        }
    }

    /// Read a single MFT record by record number
    pub fn read_record(&mut self, record_number: u64) -> Result<Vec<u8>> {
        let record_size = self.volume_data.bytes_per_file_record_segment as usize;
        let mut buffer = vec![0u8; record_size];

        let offset = self.calculate_record_offset(record_number);
        let bytes_read = read_volume_at(&self.handle, offset, &mut buffer)?;

        if bytes_read < record_size {
            return Err(RustyScanError::MftReadError(format!(
                "Short read for record {}: got {} bytes, expected {}",
                record_number, bytes_read, record_size
            )));
        }

        Ok(buffer)
    }

    /// Calculate the byte offset of an MFT record
    fn calculate_record_offset(&self, record_number: u64) -> u64 {
        let record_size = self.volume_data.bytes_per_file_record_segment as u64;

        if self.mft_extents.is_empty() {
            // Simple case: MFT is contiguous
            self.volume_data.mft_byte_offset() + record_number * record_size
        } else {
            // Complex case: MFT is fragmented
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

            // Fallback to simple calculation
            self.volume_data.mft_byte_offset() + record_number * record_size
        }
    }

    /// Parse a raw MFT record buffer into a FileEntry
    pub fn parse_record(&self, record_number: u64, data: &mut [u8]) -> Result<FileEntry> {
        // Verify signature
        let header = MftRecordHeader::from_bytes(data).ok_or_else(|| {
            RustyScanError::InvalidMftRecord(record_number, "Failed to parse header".to_string())
        })?;

        if !header.is_valid() {
            return Err(RustyScanError::InvalidMftRecord(
                record_number,
                "Invalid signature".to_string(),
            ));
        }

        // Apply fixup array (critical for data integrity!)
        self.apply_fixup(record_number, data, &header)?;

        // Create entry
        let mut entry = FileEntry {
            record_number,
            is_directory: header.is_directory(),
            is_valid: header.is_in_use(),
            hard_link_count: header.hard_link_count,
            ..Default::default()
        };

        if !entry.is_valid {
            return Ok(entry);
        }

        // Parse attributes
        self.parse_attributes(data, &header, &mut entry)?;

        entry.is_complete = true;
        Ok(entry)
    }

    /// Apply fixup array to repair sector boundaries
    ///
    /// NTFS stores the last 2 bytes of each sector in the fixup array
    /// and replaces them with a sequence number for integrity verification.
    fn apply_fixup(
        &self,
        record_number: u64,
        data: &mut [u8],
        header: &MftRecordHeader,
    ) -> Result<()> {
        let sector_size = SECTOR_SIZE as usize;
        let update_seq_offset = header.update_sequence_offset as usize;
        let update_seq_count = header.update_sequence_size as usize;

        if update_seq_offset + 2 > data.len() {
            return Err(RustyScanError::FixupVerificationFailed(record_number));
        }

        // Read sequence number (first value in update sequence array)
        let seq_number = u16::from_le_bytes([data[update_seq_offset], data[update_seq_offset + 1]]);

        // Verify and restore each sector
        for i in 1..update_seq_count {
            let sector_end = i * sector_size - 2;
            let fixup_offset = update_seq_offset + i * 2;

            if sector_end + 2 > data.len() || fixup_offset + 2 > data.len() {
                break;
            }

            // Verify sequence number at end of sector
            let stored_seq = u16::from_le_bytes([data[sector_end], data[sector_end + 1]]);
            if stored_seq != seq_number {
                return Err(RustyScanError::FixupVerificationFailed(record_number));
            }

            // Restore original bytes from fixup array
            data[sector_end] = data[fixup_offset];
            data[sector_end + 1] = data[fixup_offset + 1];
        }

        Ok(())
    }

    /// Parse all attributes in an MFT record
    fn parse_attributes(
        &self,
        data: &[u8],
        header: &MftRecordHeader,
        entry: &mut FileEntry,
    ) -> Result<()> {
        let record_size = self.volume_data.bytes_per_file_record_segment as usize;
        let mut offset = header.first_attribute_offset as usize;

        // Track best filename (prefer Win32 namespace)
        let mut best_filename: Option<(FilenameNamespace, String)> = None;

        while offset + 16 <= record_size && offset + 16 <= data.len() {
            let attr_header = AttributeHeader::from_bytes(&data[offset..]).ok_or_else(|| {
                RustyScanError::InvalidAttribute(offset as u32, "Failed to parse header".to_string())
            })?;

            // End of attributes
            if attr_header.attribute_type == ATTRIBUTE_END_MARKER || attr_header.length == 0 {
                break;
            }

            // Bounds check
            if offset + attr_header.length as usize > data.len() {
                break;
            }

            let attr_data = &data[offset..offset + attr_header.length as usize];

            match AttributeType::from_u32(attr_header.attribute_type) {
                Some(AttributeType::StandardInformation) => {
                    self.parse_standard_information(attr_data, entry)?;
                }
                Some(AttributeType::FileName) => {
                    if let Some((ns, name, parent)) = self.parse_filename(attr_data)? {
                        // Update parent reference
                        if entry.parent_record_number == 0 {
                            entry.parent_record_number = parent;
                        }

                        // Keep best filename (Win32 > Win32+DOS > POSIX > DOS)
                        let dominated = match (&best_filename, ns) {
                            (None, _) => true,
                            (Some((FilenameNamespace::Dos, _)), _) => ns != FilenameNamespace::Dos,
                            (Some((FilenameNamespace::Posix, _)), ns) => {
                                ns == FilenameNamespace::Win32 || ns == FilenameNamespace::Win32AndDos
                            }
                            (Some((FilenameNamespace::Win32AndDos, _)), ns) => {
                                ns == FilenameNamespace::Win32
                            }
                            (Some((FilenameNamespace::Win32, _)), _) => false,
                        };

                        if dominated {
                            best_filename = Some((ns, name));
                        }
                    }
                }
                Some(AttributeType::Data) => {
                    self.parse_data_attribute(attr_data, &attr_header, entry)?;
                }
                Some(AttributeType::AttributeList) => {
                    // TODO: Handle attribute lists for large files
                }
                _ => {
                    // Skip other attributes
                }
            }

            offset += attr_header.length as usize;
        }

        // Set final filename
        if let Some((_, name)) = best_filename {
            entry.name = name;
        }

        Ok(())
    }

    /// Parse $STANDARD_INFORMATION attribute
    fn parse_standard_information(&self, attr_data: &[u8], entry: &mut FileEntry) -> Result<()> {
        let header = ResidentAttributeHeader::from_bytes(attr_data);

        if let Some(h) = header {
            let content_offset = h.value_offset as usize;
            let content_len = h.value_length as usize;

            if content_offset + content_len <= attr_data.len() {
                let content = &attr_data[content_offset..content_offset + content_len];

                if let Some(si) = StandardInformation::from_bytes(content) {
                    entry.creation_time = si.creation_time;
                    entry.modification_time = si.modification_time;
                    entry.access_time = si.access_time;
                    entry.attributes = si.file_attributes;

                    // Update directory flag from attributes
                    entry.is_directory =
                        entry.is_directory || (si.file_attributes & file_attributes::DIRECTORY) != 0;
                }
            }
        }

        Ok(())
    }

    /// Parse $FILE_NAME attribute
    fn parse_filename(
        &self,
        attr_data: &[u8],
    ) -> Result<Option<(FilenameNamespace, String, u64)>> {
        let header = ResidentAttributeHeader::from_bytes(attr_data);

        if let Some(h) = header {
            let content_offset = h.value_offset as usize;
            let content_len = h.value_length as usize;

            if content_offset + content_len <= attr_data.len() {
                let content = &attr_data[content_offset..content_offset + content_len];

                if let Some(fn_attr) = FileNameAttribute::from_bytes(content) {
                    let parent_ref = fn_attr.parent_record_number();
                    return Ok(Some((fn_attr.namespace, fn_attr.name, parent_ref)));
                }
            }
        }

        Ok(None)
    }

    /// Parse $DATA attribute
    fn parse_data_attribute(
        &self,
        attr_data: &[u8],
        header: &AttributeHeader,
        entry: &mut FileEntry,
    ) -> Result<()> {
        // Check for named stream (alternate data stream)
        let stream_name = if header.name_length > 0 {
            let name_offset = header.name_offset as usize;
            let name_len = header.name_length as usize * 2;
            if name_offset + name_len <= attr_data.len() {
                let name_data = &attr_data[name_offset..name_offset + name_len];
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

        if header.non_resident {
            // Non-resident: file data is in clusters
            if let Some(nr_header) = NonResidentAttributeHeader::from_bytes(attr_data) {
                let size = nr_header.data_size;
                let allocated = nr_header.allocated_size;

                match &stream_name {
                    None => {
                        // Main data stream
                        entry.file_size = size;
                        entry.allocated_size = allocated;

                        // Decode data runs
                        let runs_offset = nr_header.data_runs_offset as usize;
                        if runs_offset < attr_data.len() {
                            let (runs, _) = DataRun::decode_runs(&attr_data[runs_offset..]);
                            entry.data_runs = runs;
                        }
                    }
                    Some(name) => {
                        // Alternate data stream
                        entry.alternate_streams.insert(name.clone(), size);
                    }
                }
            }
        } else {
            // Resident: file data is in the MFT record itself
            if let Some(r_header) = ResidentAttributeHeader::from_bytes(attr_data) {
                let size = r_header.value_length as u64;

                match &stream_name {
                    None => {
                        entry.file_size = size;
                        entry.allocated_size = 0; // Resident data doesn't use clusters
                    }
                    Some(name) => {
                        entry.alternate_streams.insert(name.clone(), size);
                    }
                }
            }
        }

        Ok(())
    }

    /// Get volume data reference
    pub fn volume_data(&self) -> &NtfsVolumeData {
        &self.volume_data
    }

    /// Get estimated total records
    pub fn estimated_records(&self) -> u64 {
        self.volume_data.estimated_mft_records()
    }
}

// ============================================================================
// Batch Reading for Performance
// ============================================================================

impl MftParser {
    /// Read multiple consecutive MFT records at once
    pub fn read_records_batch(
        &mut self,
        start_record: u64,
        count: usize,
    ) -> Result<Vec<(u64, Vec<u8>)>> {
        let record_size = self.volume_data.bytes_per_file_record_segment as usize;
        let total_size = record_size * count;

        // Resize buffer if needed
        if self.read_buffer.len() < total_size {
            self.read_buffer.resize(total_size, 0);
        }

        let offset = self.calculate_record_offset(start_record);
        let bytes_read = read_volume_at(&self.handle, offset, &mut self.read_buffer[..total_size])?;

        let records_read = bytes_read / record_size;
        let mut results = Vec::with_capacity(records_read);

        for i in 0..records_read {
            let record_offset = i * record_size;
            let record_data = self.read_buffer[record_offset..record_offset + record_size].to_vec();
            results.push((start_record + i as u64, record_data));
        }

        Ok(results)
    }
}
