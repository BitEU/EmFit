//! MFT (Master File Table) Parser
//!
//! Handles reading and parsing MFT records with fixup verification,
//! attribute extraction, and data run decoding.

use crate::error::{Result, EmFitError};
use crate::logging;
use crate::ntfs::structs::*;
use crate::ntfs::winapi::*;
use std::collections::{HashMap, HashSet};

// ============================================================================
// Parsed File Entry
// ============================================================================

/// A hard link entry - represents one $FILE_NAME attribute
#[derive(Debug, Clone)]
pub struct HardLink {
    /// Parent directory record number for this link
    pub parent_record_number: u64,
    /// File name for this link
    pub name: String,
    /// Filename namespace (Win32, DOS, POSIX, Win32AndDos)
    pub namespace: FilenameNamespace,
}

/// Complete parsed information for a file/directory
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// MFT record number
    pub record_number: u64,
    /// Parent directory record number (from best filename)
    pub parent_record_number: u64,
    /// Full file reference number (record_number | (sequence_number << 48))
    /// This is needed for OpenFileById API calls
    pub file_reference_number: u64,
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
    /// Extension MFT record numbers that may contain additional attributes (e.g., $FILE_NAME)
    pub extension_records: Vec<u64>,
    /// Extension record containing the $DATA attribute (if not in base record)
    pub data_extension_record: Option<u64>,
    /// All hard links (different $FILE_NAME attributes with different parents)
    /// Each entry represents a different location where this file appears
    pub hard_links: Vec<HardLink>,
}

impl Default for FileEntry {
    fn default() -> Self {
        Self {
            record_number: 0,
            parent_record_number: 0,
            file_reference_number: 0,
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
            extension_records: Vec::new(),
            data_extension_record: None,
            hard_links: Vec::new(),
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
    /// I/O source (volume handle or physical drive)
    io: crate::ntfs::physical::VolumeIO,
    /// NTFS volume data (cached copy for quick access)
    volume_data: NtfsVolumeData,
    /// MFT extents (for fragmented MFT)
    mft_extents: Vec<Extent>,
    /// Sector-aligned read buffer
    read_buffer: Vec<u8>,
}

impl MftParser {
    /// Create a new MFT parser from a VolumeIO source
    pub fn new(io: crate::ntfs::physical::VolumeIO) -> Result<Self> {
        let volume_data = io.volume_data().clone();
        // Allocate aligned buffer for reading
        let buffer_size = (volume_data.bytes_per_file_record_segment * 16) as usize;
        let read_buffer = vec![0u8; buffer_size];

        Ok(Self {
            io,
            volume_data,
            mft_extents: Vec::new(),
            read_buffer,
        })
    }

    /// Get number of MFT extents (0 = contiguous, >0 = fragmented)
    pub fn extent_count(&self) -> usize {
        self.mft_extents.len()
    }

    /// Get MFT extents for debugging
    pub fn extents(&self) -> &[Extent] {
        &self.mft_extents
    }

    /// Get MFT extents.
    /// In physical drive mode, always parses record 0 directly (no NTFS driver IOCTLs).
    /// In volume mode, tries opening $MFT first, then falls back to record 0 parsing.
    pub fn load_mft_extents(&mut self, drive_letter: char) -> Result<()> {
        if self.io.is_physical() {
            // Physical drive mode: parse record 0's data runs directly
            return self.load_mft_extents_from_record_zero();
        }

        // Volume mode: try FSCTL_GET_RETRIEVAL_POINTERS first
        let mft_path = format!("{}:\\$MFT", drive_letter);

        match open_file_read(&mft_path) {
            Ok(mft_handle) => {
                self.mft_extents = get_retrieval_pointers(&mft_handle, 0)?;
                Ok(())
            }
            Err(_) => {
                // If we can't open $MFT directly, parse record 0 to get MFT extents
                // Record 0 is the $MFT file itself, and its $DATA attribute contains
                // the data runs that describe where the MFT is located on disk.
                self.load_mft_extents_from_record_zero()
            }
        }
    }

    /// Load MFT extents by parsing record 0's data runs
    fn load_mft_extents_from_record_zero(&mut self) -> Result<()> {
        // Record 0 is always at the beginning of the MFT, which we can find
        // from the volume data's MftStartLcn
        let record_size = self.volume_data.bytes_per_file_record_segment as usize;
        let mft_start = self.volume_data.mft_byte_offset();

        let mut buffer = vec![0u8; record_size];
        let bytes_read = self.io.read_at(mft_start, &mut buffer)?;
        
        if bytes_read < record_size {
            return Ok(()); // Fallback: no extents, use linear calculation
        }
        
        // Parse the MFT record header
        let header = match MftRecordHeader::from_bytes(&buffer) {
            Some(h) if h.is_valid() => h,
            _ => return Ok(()), // Invalid record, use fallback
        };
        
        // Apply fixup
        if let Err(_) = self.apply_fixup(0, &mut buffer, &header) {
            return Ok(()); // Fixup failed, use fallback
        }
        
        // Parse attributes to find $DATA
        let mut offset = header.first_attribute_offset as usize;
        
        while offset + 16 <= record_size && offset + 16 <= buffer.len() {
            let attr_header = match AttributeHeader::from_bytes(&buffer[offset..]) {
                Some(h) => h,
                None => break,
            };
            
            if attr_header.attribute_type == ATTRIBUTE_END_MARKER || attr_header.length == 0 {
                break;
            }
            
            if offset + attr_header.length as usize > buffer.len() {
                break;
            }
            
            // Look for $DATA attribute (type 0x80) with no name (main data stream)
            if attr_header.attribute_type == 0x80 && attr_header.name_length == 0 && attr_header.non_resident {
                let attr_data = &buffer[offset..offset + attr_header.length as usize];
                
                if let Some(nr_header) = NonResidentAttributeHeader::from_bytes(attr_data) {
                    // Extract mft_valid_data_length from the $DATA attribute's data_size
                    // This is critical for physical drive mode where we don't have
                    // FSCTL_GET_NTFS_VOLUME_DATA to provide this value
                    if self.volume_data.mft_valid_data_length == 0 && nr_header.data_size > 0 {
                        self.volume_data.mft_valid_data_length = nr_header.data_size;
                        self.io.volume_data_mut().mft_valid_data_length = nr_header.data_size;
                        logging::info("MFT", &format!(
                            "Extracted mft_valid_data_length from record 0: {} ({} records)",
                            nr_header.data_size,
                            nr_header.data_size / self.volume_data.bytes_per_file_record_segment as u64,
                        ));
                    }

                    let runs_offset = nr_header.data_runs_offset as usize;
                    if runs_offset < attr_data.len() {
                        let (runs, _) = DataRun::decode_runs(&attr_data[runs_offset..]);

                        // Convert DataRuns to Extents
                        // Note: DataRun.lcn_offset already contains the absolute LCN
                        // (decode_runs accumulates the offsets internally)
                        let mut current_vcn: u64 = 0;

                        for run in runs {
                            if run.is_sparse {
                                current_vcn += run.cluster_count;
                                continue;
                            }

                            // lcn_offset is already the absolute LCN, not a delta
                            self.mft_extents.push(Extent {
                                vcn: current_vcn,
                                lcn: run.lcn_offset as u64,
                                cluster_count: run.cluster_count,
                            });

                            current_vcn += run.cluster_count;
                        }

                        break;
                    }
                }
            }
            
            offset += attr_header.length as usize;
        }
        
        Ok(())
    }

    /// Read a single MFT record by record number
    pub fn read_record(&mut self, record_number: u64) -> Result<Vec<u8>> {
        let record_size = self.volume_data.bytes_per_file_record_segment as usize;
        let mut buffer = vec![0u8; record_size];

        let offset = self.calculate_record_offset(record_number);
        let bytes_read = self.io.read_at(offset, &mut buffer)?;

        if bytes_read < record_size {
            return Err(EmFitError::MftReadError(format!(
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
            EmFitError::InvalidMftRecord(record_number, "Failed to parse header".to_string())
        })?;

        if !header.is_valid() {
            return Err(EmFitError::InvalidMftRecord(
                record_number,
                "Invalid signature".to_string(),
            ));
        }

        // Apply fixup array (critical for data integrity!)
        self.apply_fixup(record_number, data, &header)?;

        // Construct full file reference number (record_number | (sequence_number << 48))
        // This is needed for OpenFileById API calls
        let file_reference_number = record_number | ((header.sequence_number as u64) << 48);

        // Create entry
        let mut entry = FileEntry {
            record_number,
            file_reference_number,
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

        // Log MFT entry details
        let hard_links: Vec<(u64, String)> = entry.hard_links
            .iter()
            .map(|hl| (hl.parent_record_number, hl.name.clone()))
            .collect();

        logging::log_mft_entry(
            entry.record_number,
            entry.parent_record_number,
            entry.file_reference_number,
            &entry.name,
            entry.file_size,
            entry.allocated_size,
            entry.creation_time,
            entry.modification_time,
            entry.attributes,
            entry.is_directory,
            entry.hard_link_count,
            &entry.extension_records,
            entry.data_extension_record,
            &hard_links,
        );

        Ok(entry)
    }

    /// Parse a raw MFT record and follow extension records if needed to get the file name
    /// This version can read additional MFT records to resolve missing $FILE_NAME attributes
    pub fn parse_record_with_extensions(&mut self, record_number: u64, data: &mut [u8]) -> Result<FileEntry> {
        // First, parse the base record
        let mut entry = self.parse_record(record_number, data)?;

        // If we have extension records and no name, try to get the name from extension records
        if entry.name.is_empty() && !entry.extension_records.is_empty() && entry.is_valid {
            // Read extension records to find $FILE_NAME
            let extension_records = std::mem::take(&mut entry.extension_records);

            for ext_record_num in extension_records {
                // Skip if it's the same as the base record
                if ext_record_num == record_number {
                    continue;
                }

                // Try to read and parse the extension record
                match self.read_record(ext_record_num) {
                    Ok(mut ext_data) => {
                        if let Some((name, parent)) = self.extract_filename_from_extension(&mut ext_data) {
                            if !name.is_empty() {
                                entry.name = name;
                                if entry.parent_record_number == 0 {
                                    entry.parent_record_number = parent;
                                }
                                break; // Found a name, we're done
                            }
                        }
                    }
                    Err(_) => {
                        // Failed to read extension record, continue trying others
                        continue;
                    }
                }
            }
        }

        Ok(entry)
    }

    /// Extract just the file name from an extension MFT record
    fn extract_filename_from_extension(&self, data: &mut [u8]) -> Option<(String, u64)> {
        // Parse header
        let header = MftRecordHeader::from_bytes(data)?;

        if !header.is_valid() {
            return None;
        }

        // Apply fixup
        if self.apply_fixup(0, data, &header).is_err() {
            return None;
        }

        // Look for $FILE_NAME attribute
        let record_size = self.volume_data.bytes_per_file_record_segment as usize;
        let mut offset = header.first_attribute_offset as usize;
        let mut best_name: Option<(String, u64, FilenameNamespace)> = None;

        while offset + 16 <= record_size && offset + 16 <= data.len() {
            let attr_header = match AttributeHeader::from_bytes(&data[offset..]) {
                Some(h) => h,
                None => break,
            };

            if attr_header.attribute_type == ATTRIBUTE_END_MARKER || attr_header.length == 0 {
                break;
            }

            if offset + attr_header.length as usize > data.len() {
                break;
            }

            // Look for FILE_NAME attribute (type 0x30)
            if attr_header.attribute_type == 0x30 && !attr_header.non_resident {
                let attr_data = &data[offset..offset + attr_header.length as usize];

                if let Some(res_header) = ResidentAttributeHeader::from_bytes(attr_data) {
                    let content_offset = res_header.value_offset as usize;
                    let content_len = res_header.value_length as usize;

                    if content_offset + content_len <= attr_data.len() {
                        let content = &attr_data[content_offset..content_offset + content_len];

                        if let Some(fn_attr) = FileNameAttribute::from_bytes(content) {
                            let parent = fn_attr.parent_record_number();
                            let ns = fn_attr.namespace;

                            // Prefer Win32 or Win32+DOS namespace
                            let dominated = match &best_name {
                                None => true,
                                Some((_, _, existing_ns)) => {
                                    match (ns, existing_ns) {
                                        (FilenameNamespace::Win32, _) => true,
                                        (FilenameNamespace::Win32AndDos, FilenameNamespace::Dos) => true,
                                        (FilenameNamespace::Win32AndDos, FilenameNamespace::Posix) => true,
                                        (FilenameNamespace::Posix, FilenameNamespace::Dos) => true,
                                        _ => false,
                                    }
                                }
                            };

                            if dominated {
                                best_name = Some((fn_attr.name.clone(), parent, ns));
                            }
                        }
                    }
                }
            }

            offset += attr_header.length as usize;
        }

        best_name.map(|(name, parent, _)| (name, parent))
    }

    /// Extract ALL $FILE_NAME attributes from an extension MFT record.
    ///
    /// Unlike `extract_filename_from_extension` which returns only the "best" name,
    /// this returns every non-DOS filename as a HardLink. This is critical for
    /// discovering hard links whose $FILE_NAME attributes spilled into extension records.
    fn extract_all_filenames_from_extension(&self, data: &mut [u8]) -> Vec<HardLink> {
        let mut links = Vec::new();

        // Parse header
        let header = match MftRecordHeader::from_bytes(data) {
            Some(h) => h,
            None => return links,
        };

        if !header.is_valid() {
            return links;
        }

        // Apply fixup
        if self.apply_fixup(0, data, &header).is_err() {
            return links;
        }

        let record_size = self.volume_data.bytes_per_file_record_segment as usize;
        let mut offset = header.first_attribute_offset as usize;

        while offset + 16 <= record_size && offset + 16 <= data.len() {
            let attr_header = match AttributeHeader::from_bytes(&data[offset..]) {
                Some(h) => h,
                None => break,
            };

            if attr_header.attribute_type == ATTRIBUTE_END_MARKER || attr_header.length == 0 {
                break;
            }

            if offset + attr_header.length as usize > data.len() {
                break;
            }

            // Look for FILE_NAME attribute (type 0x30)
            if attr_header.attribute_type == 0x30 && !attr_header.non_resident {
                let attr_data = &data[offset..offset + attr_header.length as usize];

                if let Some(res_header) = ResidentAttributeHeader::from_bytes(attr_data) {
                    let content_offset = res_header.value_offset as usize;
                    let content_len = res_header.value_length as usize;

                    if content_offset + content_len <= attr_data.len() {
                        let content = &attr_data[content_offset..content_offset + content_len];

                        if let Some(fn_attr) = FileNameAttribute::from_bytes(content) {
                            // Skip DOS-only names (short 8.3 aliases, not real hard links)
                            if fn_attr.namespace != FilenameNamespace::Dos {
                                links.push(HardLink {
                                    parent_record_number: fn_attr.parent_record_number(),
                                    name: fn_attr.name.clone(),
                                    namespace: fn_attr.namespace,
                                });
                            }
                        }
                    }
                }
            }

            offset += attr_header.length as usize;
        }

        links
    }

    /// Extract file size from an extension MFT record containing $DATA attribute
    ///
    /// Returns (file_size, allocated_size) if found
    fn extract_size_from_extension(&self, data: &mut [u8]) -> Option<(u64, u64)> {
        // Parse header
        let header = MftRecordHeader::from_bytes(data)?;

        if !header.is_valid() {
            return None;
        }

        // Apply fixup
        if self.apply_fixup(0, data, &header).is_err() {
            return None;
        }

        // Look for $DATA attribute (type 0x80)
        let record_size = self.volume_data.bytes_per_file_record_segment as usize;
        let mut offset = header.first_attribute_offset as usize;

        while offset + 16 <= record_size && offset + 16 <= data.len() {
            let attr_header = match AttributeHeader::from_bytes(&data[offset..]) {
                Some(h) => h,
                None => break,
            };

            if attr_header.attribute_type == ATTRIBUTE_END_MARKER || attr_header.length == 0 {
                break;
            }

            if offset + attr_header.length as usize > data.len() {
                break;
            }

            // Look for $DATA attribute (type 0x80) with no name (unnamed = default stream)
            if attr_header.attribute_type == 0x80 && attr_header.name_length == 0 {
                let attr_data = &data[offset..offset + attr_header.length as usize];

                if attr_header.non_resident {
                    // Non-resident $DATA - size is in the non-resident header
                    if let Some(nr_header) = NonResidentAttributeHeader::from_bytes(attr_data) {
                        // Only process if this is the first extent (lowest VCN == 0)
                        if nr_header.lowest_vcn == 0 {
                            return Some((nr_header.data_size, nr_header.allocated_size));
                        }
                    }
                } else {
                    // Resident $DATA - size is the content length
                    if let Some(r_header) = ResidentAttributeHeader::from_bytes(attr_data) {
                        let size = r_header.value_length as u64;
                        return Some((size, 0)); // Resident has no allocated clusters
                    }
                }
            }

            offset += attr_header.length as usize;
        }

        None
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
            return Err(EmFitError::FixupVerificationFailed(record_number));
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
                return Err(EmFitError::FixupVerificationFailed(record_number));
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

        // Track best filename (prefer Win32 namespace) for the primary name/parent
        // Tuple: (namespace, name, parent, creation_time, modification_time)
        let mut best_filename: Option<(FilenameNamespace, String, u64, u64, u64)> = None;

        // Collect ALL $FILE_NAME attributes for hard link support
        // Each $FILE_NAME can have a different parent directory (hard link)
        let mut all_filenames: Vec<(FilenameNamespace, String, u64)> = Vec::new();

        // Track extension record numbers we need to read for $FILE_NAME
        let mut extension_records: Vec<u64> = Vec::new();

        while offset + 16 <= record_size && offset + 16 <= data.len() {
            let attr_header = AttributeHeader::from_bytes(&data[offset..]).ok_or_else(|| {
                EmFitError::InvalidAttribute(offset as u32, "Failed to parse header".to_string())
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
                    if let Some((ns, name, parent, ctime, mtime)) = self.parse_filename(attr_data)? {
                        // Skip DOS-only names (like Everything does)
                        // DOS names are short 8.3 aliases, not real hard links
                        if ns != FilenameNamespace::Dos {
                            // Collect ALL non-DOS filenames for hard link tracking
                            all_filenames.push((ns, name.clone(), parent));
                        }

                        // Keep best filename for primary entry (Win32 > Win32+DOS > POSIX > DOS)
                        let dominated = match (&best_filename, ns) {
                            (None, _) => true,
                            (Some((FilenameNamespace::Dos, _, ..)), _) => ns != FilenameNamespace::Dos,
                            (Some((FilenameNamespace::Posix, _, ..)), ns) => {
                                ns == FilenameNamespace::Win32 || ns == FilenameNamespace::Win32AndDos
                            }
                            (Some((FilenameNamespace::Win32AndDos, _, ..)), ns) => {
                                ns == FilenameNamespace::Win32
                            }
                            (Some((FilenameNamespace::Win32, _, ..)), _) => false,
                        };

                        if dominated {
                            best_filename = Some((ns, name, parent, ctime, mtime));
                        }
                    }
                }
                Some(AttributeType::Data) => {
                    self.parse_data_attribute(attr_data, &attr_header, entry)?;
                }
                Some(AttributeType::AttributeList) => {
                    // Parse the attribute list to find extension records with $FILE_NAME and $DATA
                    let (filename_ext_records, data_ext_record) = self.parse_attribute_list(attr_data, entry.record_number)?;
                    extension_records.extend(filename_ext_records);
                    if data_ext_record.is_some() {
                        entry.data_extension_record = data_ext_record;
                    }
                }
                _ => {
                    // Skip other attributes
                }
            }

            offset += attr_header.length as usize;
        }

        // Set primary filename and parent from best match
        if let Some((_, name, parent, fn_ctime, fn_mtime)) = best_filename {
            entry.name = name;
            entry.parent_record_number = parent;

            // Fallback: if $STANDARD_INFORMATION timestamps are missing (0),
            // use $FILE_NAME timestamps instead. This handles cases where
            // $STANDARD_INFORMATION is corrupt or missing.
            if entry.creation_time == 0 && fn_ctime != 0 {
                entry.creation_time = fn_ctime;
            }
            if entry.modification_time == 0 && fn_mtime != 0 {
                entry.modification_time = fn_mtime;
            }
        }

        // Convert all filenames to HardLink entries
        // This allows creating separate tree entries for each hard link location
        entry.hard_links = all_filenames
            .into_iter()
            .map(|(ns, name, parent)| HardLink {
                parent_record_number: parent,
                name,
                namespace: ns,
            })
            .collect();

        // Store extension records for later processing by the caller
        entry.extension_records = extension_records;

        Ok(())
    }

    /// Parse an Attribute List to find extension records containing important attributes
    ///
    /// Returns a tuple of (filename_extension_records, data_extension_record)
    /// - filename_extension_records: Records containing $FILE_NAME (0x30)
    /// - data_extension_record: Record containing the primary $DATA (0x80) attribute with VCN 0
    fn parse_attribute_list(
        &self,
        attr_data: &[u8],
        base_record_number: u64,
    ) -> Result<(Vec<u64>, Option<u64>)> {
        // Get the attribute list content
        let attr_header = AttributeHeader::from_bytes(attr_data).ok_or_else(|| {
            EmFitError::InvalidAttribute(0, "Failed to parse attr list header".to_string())
        })?;

        let list_data = if attr_header.non_resident {
            // Non-resident attribute list - we'd need to read the data runs
            // This is rare for attribute lists, skip for now
            return Ok((Vec::new(), None));
        } else {
            // Resident - get the content directly
            let res_header = ResidentAttributeHeader::from_bytes(attr_data).ok_or_else(|| {
                EmFitError::InvalidAttribute(0, "Failed to parse resident header".to_string())
            })?;

            let content_offset = res_header.value_offset as usize;
            let content_len = res_header.value_length as usize;

            if content_offset + content_len > attr_data.len() {
                return Ok((Vec::new(), None));
            }

            &attr_data[content_offset..content_offset + content_len]
        };

        // Parse the attribute list entries
        let entries = parse_attribute_list(list_data);

        // Find extension records that contain $FILE_NAME (type 0x30)
        let mut filename_extension_records = Vec::new();
        // Find extension record containing $DATA (type 0x80) with starting VCN 0 (primary data stream)
        let mut data_extension_record: Option<u64> = None;

        for entry in entries {
            let ext_record = entry.record_number();

            // Look for FILE_NAME attributes in extension records
            if entry.attribute_type == 0x30 {
                if ext_record != base_record_number && !filename_extension_records.contains(&ext_record) {
                    filename_extension_records.push(ext_record);
                }
            }

            // Look for primary $DATA attribute (type 0x80, starting VCN 0, no name = unnamed default stream)
            // Like Everything does at line 97933 and 98028
            if entry.attribute_type == 0x80 && entry.starting_vcn == 0 && entry.name_length == 0 {
                if ext_record != base_record_number && data_extension_record.is_none() {
                    data_extension_record = Some(ext_record);
                }
            }
        }

        Ok((filename_extension_records, data_extension_record))
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
    /// Returns (namespace, name, parent_record_number, creation_time, modification_time)
    fn parse_filename(
        &self,
        attr_data: &[u8],
    ) -> Result<Option<(FilenameNamespace, String, u64, u64, u64)>> {
        let header = ResidentAttributeHeader::from_bytes(attr_data);

        if let Some(h) = header {
            let content_offset = h.value_offset as usize;
            let content_len = h.value_length as usize;

            if content_offset + content_len <= attr_data.len() {
                let content = &attr_data[content_offset..content_offset + content_len];

                if let Some(fn_attr) = FileNameAttribute::from_bytes(content) {
                    let parent_ref = fn_attr.parent_record_number();
                    return Ok(Some((
                        fn_attr.namespace,
                        fn_attr.name,
                        parent_ref,
                        fn_attr.creation_time,
                        fn_attr.modification_time,
                    )));
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

    /// Get MFT extents (cloned, for creating MftRecordFetcher)
    pub fn mft_extents(&self) -> Vec<Extent> {
        self.mft_extents.clone()
    }

    /// Check if the parser is using physical drive mode
    pub fn is_physical(&self) -> bool {
        self.io.is_physical()
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
        let bytes_read = self.io.read_at(offset, &mut self.read_buffer[..total_size])?;

        let records_read = bytes_read / record_size;
        let mut results = Vec::with_capacity(records_read);

        for i in 0..records_read {
            let record_offset = i * record_size;
            let record_data = self.read_buffer[record_offset..record_offset + record_size].to_vec();
            results.push((start_record + i as u64, record_data));
        }

        Ok(results)
    }

    /// Parse a batch of MFT records, efficiently resolving extension records for missing file names and sizes.
    ///
    /// This is a multi-pass algorithm:
    /// 1. First pass: Parse all records, collecting those that need extension record resolution
    /// 2. Second pass: Batch-read all needed extension records and resolve missing names/sizes
    ///
    /// This is more efficient than reading extension records one-by-one as it minimizes disk seeks.
    pub fn parse_batch_with_extensions(
        &mut self,
        batch: Vec<(u64, Vec<u8>)>,
    ) -> Vec<FileEntry> {
        let mut entries: Vec<FileEntry> = Vec::with_capacity(batch.len());
        let mut needs_name_extension: Vec<usize> = Vec::new(); // Indices into entries that need name resolution
        let mut needs_hardlink_extension: Vec<usize> = Vec::new(); // Indices that have extension records with potential additional hard links
        let mut needs_data_extension: Vec<usize> = Vec::new(); // Indices into entries that need size resolution
        let mut extension_record_set: HashSet<u64> = HashSet::new(); // All extension records we need to read

        // First pass: Parse all records
        for (record_num, mut data) in batch {
            match self.parse_record(record_num, &mut data) {
                Ok(entry) => {
                    if entry.is_valid {
                        let idx = entries.len();

                        if !entry.extension_records.is_empty() {
                            // Always collect extension records that contain $FILE_NAME attributes
                            // These may hold additional hard links even if the base record already has a name
                            for ext_rec in &entry.extension_records {
                                if *ext_rec != record_num {
                                    extension_record_set.insert(*ext_rec);
                                }
                            }

                            if entry.name.is_empty() {
                                // Entry has no name at all - needs primary name resolution
                                needs_name_extension.push(idx);
                            } else {
                                // Entry already has a name but may have additional hard links in extension records
                                needs_hardlink_extension.push(idx);
                            }
                        }

                        // Check if this entry needs $DATA extension record resolution
                        // (file_size == 0 and we have a data extension record to read)
                        if entry.file_size == 0 && !entry.is_directory {
                            if let Some(data_ext_rec) = entry.data_extension_record {
                                if data_ext_rec != record_num {
                                    extension_record_set.insert(data_ext_rec);
                                    needs_data_extension.push(idx);
                                }
                            }
                        }

                        entries.push(entry);
                    }
                }
                Err(_) => {
                    // Skip invalid records
                    continue;
                }
            }
        }

        // If no entries need extension resolution, we're done
        if needs_name_extension.is_empty() && needs_hardlink_extension.is_empty() && needs_data_extension.is_empty() {
            return entries;
        }

        // Second pass: Read all extension records we need
        // Group them by proximity for efficient reading
        let mut extension_records: Vec<u64> = extension_record_set.into_iter().collect();
        extension_records.sort_unstable();

        // Read extension records and extract filenames, hard links, and sizes
        let mut extension_names: HashMap<u64, (String, u64)> = HashMap::new();
        let mut extension_hardlinks: HashMap<u64, Vec<HardLink>> = HashMap::new();
        let mut extension_sizes: HashMap<u64, (u64, u64)> = HashMap::new(); // record -> (file_size, allocated_size)

        for ext_record_num in extension_records {
            match self.read_record(ext_record_num) {
                Ok(mut ext_data) => {
                    // Extract ALL filenames for hard link discovery
                    let all_links = self.extract_all_filenames_from_extension(&mut ext_data);
                    if !all_links.is_empty() {
                        // Also set the "best" name for primary name resolution
                        // (use the first non-DOS link as fallback)
                        if !extension_names.contains_key(&ext_record_num) {
                            let best = &all_links[0];
                            extension_names.insert(ext_record_num, (best.name.clone(), best.parent_record_number));
                        }
                        extension_hardlinks.insert(ext_record_num, all_links);
                    }
                    // Extract file size if available
                    // Note: need to re-read since extract_all_filenames_from_extension consumed the fixup
                    match self.read_record(ext_record_num) {
                        Ok(mut ext_data2) => {
                            if let Some((file_size, allocated_size)) = self.extract_size_from_extension(&mut ext_data2) {
                                extension_sizes.insert(ext_record_num, (file_size, allocated_size));
                            }
                        }
                        Err(_) => {}
                    }
                }
                Err(_) => {
                    // Failed to read extension record, continue
                    continue;
                }
            }
        }

        // Third pass: Apply extension record names to entries that need them
        for idx in needs_name_extension {
            let entry = &mut entries[idx];

            // Try each extension record for this entry until we find a name
            for ext_rec in &entry.extension_records.clone() {
                if let Some((name, parent)) = extension_names.get(ext_rec) {
                    entry.name = name.clone();
                    if entry.parent_record_number == 0 {
                        entry.parent_record_number = *parent;
                    }
                    // Also add all hard links from extension records
                    if let Some(links) = extension_hardlinks.get(ext_rec) {
                        for link in links {
                            // Avoid duplicates
                            if !entry.hard_links.iter().any(|hl| hl.parent_record_number == link.parent_record_number && hl.name == link.name) {
                                entry.hard_links.push(link.clone());
                            }
                        }
                    }
                    break; // Found a name, we're done with primary name
                }
            }
        }

        // Fourth pass: Apply additional hard links from extension records to entries that already have a name
        for idx in needs_hardlink_extension {
            let entry = &mut entries[idx];

            for ext_rec in &entry.extension_records.clone() {
                if let Some(links) = extension_hardlinks.get(ext_rec) {
                    for link in links {
                        // Avoid duplicates with existing hard links
                        if !entry.hard_links.iter().any(|hl| hl.parent_record_number == link.parent_record_number && hl.name == link.name) {
                            entry.hard_links.push(link.clone());
                        }
                    }
                }
            }
        }

        // Fifth pass: Apply extension record sizes to entries that need them
        for idx in needs_data_extension {
            let entry = &mut entries[idx];

            if let Some(data_ext_rec) = entry.data_extension_record {
                if let Some(&(file_size, allocated_size)) = extension_sizes.get(&data_ext_rec) {
                    entry.file_size = file_size;
                    entry.allocated_size = allocated_size;
                }
            }
        }

        // Clear extension_records from all entries (no longer needed)
        for entry in &mut entries {
            entry.extension_records.clear();
            entry.data_extension_record = None;
        }

        entries
    }
}

// ============================================================================
// Standalone Functions for Parent Resolution
// ============================================================================

/// Extract just the file name and parent record number from a raw MFT record.
///
/// This is a lightweight function used for on-demand parent resolution when
/// building file paths. It doesn't need an MftParser instance since it receives
/// data from FSCTL_GET_NTFS_FILE_RECORD.
///
/// Returns (name, parent_record_number) or None if the record is invalid.
pub fn extract_parent_info(data: &[u8]) -> Option<(String, u64)> {
    extract_parent_info_internal(data, false)
}

/// Debug version with verbose output
pub fn extract_parent_info_debug(data: &[u8]) -> Option<(String, u64)> {
    extract_parent_info_internal(data, true)
}

fn extract_parent_info_internal(data: &[u8], debug: bool) -> Option<(String, u64)> {
    // Parse and validate MFT record header
    let header = MftRecordHeader::from_bytes(data)?;

    if debug {
        eprintln!("    [extract] Header: sig={:08X}, flags={:04X}, first_attr={}, in_use={}",
            header.signature, header.flags, header.first_attribute_offset, header.is_in_use());
    }

    if !header.is_valid() {
        if debug { eprintln!("    [extract] Invalid signature"); }
        return None;
    }
    if !header.is_in_use() {
        if debug { eprintln!("    [extract] Record not in use"); }
        return None;
    }

    // Note: Data from FSCTL_GET_NTFS_FILE_RECORD does NOT need fixup applied.
    // Fixup is only needed when reading raw sectors from disk.
    // The Windows API returns already-valid record data.

    // Find the best FILE_NAME attribute
    let mut best_name: Option<(String, u64, FilenameNamespace)> = None;
    let mut offset = header.first_attribute_offset as usize;
    let record_size = data.len();
    let mut attr_count = 0;

    while offset + 16 <= record_size {
        let attr_header = match AttributeHeader::from_bytes(&data[offset..]) {
            Some(h) => h,
            None => {
                if debug { eprintln!("    [extract] Failed to parse attr header at offset {}", offset); }
                break;
            }
        };

        if debug {
            eprintln!("    [extract] Attr[{}] at {}: type=0x{:X}, len={}, non_res={}",
                attr_count, offset, attr_header.attribute_type, attr_header.length, attr_header.non_resident);
        }

        if attr_header.attribute_type == ATTRIBUTE_END_MARKER || attr_header.length == 0 {
            if debug { eprintln!("    [extract] End marker or zero length"); }
            break;
        }

        if offset + attr_header.length as usize > record_size {
            if debug { eprintln!("    [extract] Attr extends past record"); }
            break;
        }

        // Look for FILE_NAME attribute (type 0x30)
        if attr_header.attribute_type == 0x30 && !attr_header.non_resident {
            let attr_data = &data[offset..offset + attr_header.length as usize];

            if let Some(res_header) = ResidentAttributeHeader::from_bytes(attr_data) {
                let content_offset = res_header.value_offset as usize;
                let content_len = res_header.value_length as usize;

                if debug {
                    eprintln!("    [extract] FILE_NAME: content_offset={}, content_len={}, attr_len={}",
                        content_offset, content_len, attr_data.len());
                }

                if content_offset + content_len <= attr_data.len() {
                    let content = &attr_data[content_offset..content_offset + content_len];

                    if let Some(fn_attr) = FileNameAttribute::from_bytes(content) {
                        let parent = fn_attr.parent_record_number();
                        let ns = fn_attr.namespace;

                        if debug {
                            eprintln!("    [extract] Found name='{}', parent={}, namespace={:?}",
                                fn_attr.name, parent, ns);
                        }

                        // Prefer Win32 or Win32+DOS namespace over DOS-only
                        let dominated = match &best_name {
                            None => true,
                            Some((_, _, existing_ns)) => {
                                match (ns, existing_ns) {
                                    (FilenameNamespace::Win32, _) => true,
                                    (FilenameNamespace::Win32AndDos, FilenameNamespace::Dos) => true,
                                    (FilenameNamespace::Win32AndDos, FilenameNamespace::Posix) => true,
                                    (FilenameNamespace::Posix, FilenameNamespace::Dos) => true,
                                    _ => false,
                                }
                            }
                        };

                        if dominated {
                            best_name = Some((fn_attr.name.clone(), parent, ns));
                        }
                    } else if debug {
                        eprintln!("    [extract] FileNameAttribute::from_bytes failed");
                    }
                } else if debug {
                    eprintln!("    [extract] Content extends past attr");
                }
            } else if debug {
                eprintln!("    [extract] ResidentAttributeHeader::from_bytes failed");
            }
        }

        offset += attr_header.length as usize;
        attr_count += 1;
    }

    if debug {
        eprintln!("    [extract] Final result: {:?}", best_name.as_ref().map(|(n, p, _)| (n.as_str(), *p)));
    }

    best_name.map(|(name, parent, _)| (name, parent))
}

/// Apply fixup array to MFT record data (standalone version)
pub fn apply_fixup_standalone(data: &mut [u8], header: &MftRecordHeader) -> Result<()> {
    let fixup_offset = header.update_sequence_offset as usize;
    let fixup_count = header.update_sequence_size as usize;

    if fixup_count < 2 || fixup_offset + fixup_count * 2 > data.len() {
        return Err(EmFitError::InvalidMftRecord(0, "Invalid fixup".to_string()));
    }

    // First 2 bytes of fixup array is the check value
    let check_value = u16::from_le_bytes([data[fixup_offset], data[fixup_offset + 1]]);

    // Each subsequent pair replaces the last 2 bytes of each sector
    let sector_size = 512usize;

    for i in 1..fixup_count {
        let sector_end = (i * sector_size) - 2;

        if sector_end + 2 > data.len() {
            break;
        }

        // Verify the sector ends with the check value
        let sector_value = u16::from_le_bytes([data[sector_end], data[sector_end + 1]]);
        if sector_value != check_value {
            return Err(EmFitError::InvalidMftRecord(0, "Fixup mismatch".to_string()));
        }

        // Replace with original bytes from fixup array
        let fixup_pos = fixup_offset + i * 2;
        data[sector_end] = data[fixup_pos];
        data[sector_end + 1] = data[fixup_pos + 1];
    }

    Ok(())
}
