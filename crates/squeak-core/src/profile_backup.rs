//! Onboard profile sector memory backup, restore verification, and persistent storage.
//!
//! Excludes per-unit serial numbers and unit IDs; captures exact sector bytes and validates
//! 16-bit CCITT CRCs for each sector.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use hidpp::calculate_sector_crc;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// File format magic identifying Squeak onboard profile backup files.
pub const BACKUP_MAGIC_V1: &str = "SQUEAK_ONBOARD_BACKUP_V1";

/// Errors encountered while creating, verifying, or loading profile backups.
#[derive(Debug, Error)]
pub enum ProfileBackupError {
    /// File system I/O error.
    #[error("I/O error for {path}: {source}")]
    Io {
        /// Affected file or directory path.
        path: PathBuf,
        /// Underlying OS I/O error.
        #[source]
        source: std::io::Error,
    },

    /// JSON serialization or deserialization error.
    #[error("JSON error: {source}")]
    Json {
        /// Underlying serde_json error.
        #[source]
        source: serde_json::Error,
    },

    /// File header magic does not match [`BACKUP_MAGIC_V1`].
    #[error("invalid backup header: expected '{expected}', found '{found}'")]
    InvalidHeader {
        /// Expected header string.
        expected: &'static str,
        /// Header string found in file.
        found: String,
    },

    /// Backup was created for a different device VID:PID.
    #[error(
        "device VID:PID mismatch: backup is for 0x{backup_vid:04X}:0x{backup_pid:04X}, device is 0x{device_vid:04X}:0x{device_pid:04X}"
    )]
    DeviceMismatch {
        /// Vendor ID in backup.
        backup_vid: u16,
        /// Product ID in backup.
        backup_pid: u16,
        /// Target device Vendor ID.
        device_vid: u16,
        /// Target device Product ID.
        device_pid: u16,
    },

    /// A sector payload does not match expected sector size.
    #[error("sector 0x{sector:04X} size mismatch: expected {expected} bytes, found {found}")]
    SectorSizeMismatch {
        /// Sector address.
        sector: u16,
        /// Expected byte length.
        expected: usize,
        /// Actual byte length.
        found: usize,
    },

    /// A sector's stored CRC does not match the calculated CRC.
    #[error(
        "sector 0x{sector:04X} CRC mismatch: stored 0x{stored:04X}, calculated 0x{calculated:04X}"
    )]
    CrcMismatch {
        /// Sector address.
        sector: u16,
        /// Stored CRC in the last 2 bytes.
        stored: u16,
        /// CRC calculated over preceding bytes.
        calculated: u16,
    },

    /// Backup is missing sector 0 (directory sector).
    #[error("backup is missing sector 0x0000 (profile directory)")]
    MissingDirectorySector,
}

/// Persistent backup container for device onboard memory sectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileBackup {
    /// File magic header ([`BACKUP_MAGIC_V1`]).
    pub magic: String,
    /// Backup creation timestamp (e.g. ISO 8601 UTC string).
    pub timestamp_utc: String,
    /// Vendor ID (e.g. `0x046D`).
    pub vendor_id: u16,
    /// Product ID (e.g. `0xC08B`).
    pub product_id: u16,
    /// Byte length of each memory sector (e.g. `255`).
    pub sector_size: u16,
    /// Total sectors on the device (e.g. `16`).
    pub sector_count: u8,
    /// Map of sector address to raw sector bytes.
    pub sectors: BTreeMap<u16, Vec<u8>>,
}

impl ProfileBackup {
    /// Creates a new profile backup object.
    #[must_use]
    pub fn new(
        timestamp_utc: String,
        vendor_id: u16,
        product_id: u16,
        sector_size: u16,
        sector_count: u8,
        sectors: BTreeMap<u16, Vec<u8>>,
    ) -> Self {
        Self {
            magic: BACKUP_MAGIC_V1.to_string(),
            timestamp_utc,
            vendor_id,
            product_id,
            sector_size,
            sector_count,
            sectors,
        }
    }

    /// Verifies that all sectors in the backup have the expected size and valid CRCs.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileBackupError`] if any sector is malformed, missing, or has invalid CRC.
    pub fn verify_integrity(&self) -> Result<(), ProfileBackupError> {
        if self.magic != BACKUP_MAGIC_V1 {
            return Err(ProfileBackupError::InvalidHeader {
                expected: BACKUP_MAGIC_V1,
                found: self.magic.clone(),
            });
        }

        if !self.sectors.contains_key(&0) {
            return Err(ProfileBackupError::MissingDirectorySector);
        }

        let expected_size = usize::from(self.sector_size);
        for (&sector, data) in &self.sectors {
            if data.len() != expected_size {
                return Err(ProfileBackupError::SectorSizeMismatch {
                    sector,
                    expected: expected_size,
                    found: data.len(),
                });
            }

            if expected_size >= 2 {
                let data_len = expected_size - 2;
                let stored_crc = u16::from_be_bytes([data[data_len], data[data_len + 1]]);
                let calculated_crc = calculate_sector_crc(&data[..data_len]);
                if stored_crc != calculated_crc {
                    return Err(ProfileBackupError::CrcMismatch {
                        sector,
                        stored: stored_crc,
                        calculated: calculated_crc,
                    });
                }
            }
        }

        Ok(())
    }

    /// Serializes the backup into pretty-printed JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileBackupError::Json`] on serialization failure.
    pub fn to_json(&self) -> Result<String, ProfileBackupError> {
        serde_json::to_string_pretty(self).map_err(|source| ProfileBackupError::Json { source })
    }

    /// Deserializes a backup from JSON and verifies its integrity.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileBackupError`] if deserialization fails or CRC checks fail.
    pub fn from_json(json: &str) -> Result<Self, ProfileBackupError> {
        let backup: Self =
            serde_json::from_str(json).map_err(|source| ProfileBackupError::Json { source })?;
        backup.verify_integrity()?;
        Ok(backup)
    }

    /// Saves the backup JSON to disk, creating parent directories if needed.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileBackupError`] if writing fails.
    pub fn save_to_file(&self, path: &Path) -> Result<(), ProfileBackupError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| ProfileBackupError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let json = self.to_json()?;
        fs::write(path, json.as_bytes()).map_err(|source| ProfileBackupError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(())
    }

    /// Loads a backup JSON file from disk and validates its integrity.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileBackupError`] if reading fails or integrity check fails.
    pub fn load_from_file(path: &Path) -> Result<Self, ProfileBackupError> {
        let content = fs::read_to_string(path).map_err(|source| ProfileBackupError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_json(&content)
    }
}

/// Generates a standard path under `XDG_DATA_HOME` (or `~/.local/share/squeak/backups`) for a device.
#[must_use]
pub fn default_backup_path(vendor_id: u16, product_id: u16, timestamp: &str) -> PathBuf {
    let sanitized_ts = timestamp.replace([':', '-'], "").replace(' ', "_");
    let base_dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            home.join(".local").join("share")
        });
    base_dir
        .join("squeak")
        .join("backups")
        .join(format!("{vendor_id:04x}_{product_id:04x}"))
        .join(format!("backup_{sanitized_ts}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_backup_roundtrip_and_crc_verification() {
        let mut sectors = BTreeMap::new();

        // Sector 0 (valid CRC 0x8601)
        let mut s0 = vec![0xFF; 255];
        s0[0..4].copy_from_slice(&[0x00, 0x01, 0x01, 0xFF]);
        s0[4..8].copy_from_slice(&[0x00, 0x02, 0x01, 0xFF]);
        s0[8..12].copy_from_slice(&[0x00, 0x03, 0x01, 0xFF]);
        s0[12..16].copy_from_slice(&[0x00, 0x04, 0x01, 0xFF]);
        s0[16..20].copy_from_slice(&[0x00, 0x05, 0x01, 0xFF]);
        s0[253..255].copy_from_slice(&0x8601u16.to_be_bytes());
        sectors.insert(0, s0);

        // Sector 1 (valid CRC)
        let mut s1 = vec![0xFF; 255];
        s1[0] = 1;
        s1[1] = 1;
        let crc1 = calculate_sector_crc(&s1[..253]);
        s1[253..255].copy_from_slice(&crc1.to_be_bytes());
        sectors.insert(1, s1);

        let backup = ProfileBackup::new(
            "2026-11-13T12:00:00Z".to_string(),
            0x046D,
            0xC08B,
            255,
            16,
            sectors,
        );

        assert!(backup.verify_integrity().is_ok());

        let json = backup.to_json().unwrap();
        let loaded = ProfileBackup::from_json(&json).unwrap();
        assert_eq!(loaded, backup);
    }

    #[test]
    fn test_profile_backup_rejects_corrupted_crc() {
        let mut sectors = BTreeMap::new();
        let mut s0 = vec![0xFF; 255];
        s0[253..255].copy_from_slice(&0x1234u16.to_be_bytes()); // Wrong CRC
        sectors.insert(0, s0);

        let backup = ProfileBackup::new(
            "2026-11-13T12:00:00Z".to_string(),
            0x046D,
            0xC08B,
            255,
            16,
            sectors,
        );

        let res = backup.verify_integrity();
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ProfileBackupError::CrcMismatch { .. }
        ));
    }

    #[test]
    fn test_profile_backup_rejects_missing_sector_zero() {
        let mut sectors = BTreeMap::new();
        let mut s1 = vec![0xFF; 255];
        let crc = calculate_sector_crc(&s1[..253]);
        s1[253..255].copy_from_slice(&crc.to_be_bytes());
        sectors.insert(1, s1); // Only sector 1, missing sector 0

        let backup = ProfileBackup::new(
            "2026-11-13T12:00:00Z".to_string(),
            0x046D,
            0xC08B,
            255,
            16,
            sectors,
        );

        let res = backup.verify_integrity();
        assert!(res.is_err());
        assert!(matches!(
            res.unwrap_err(),
            ProfileBackupError::MissingDirectorySector
        ));
    }
}
