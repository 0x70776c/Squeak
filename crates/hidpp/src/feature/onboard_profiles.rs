//! `ONBOARD_PROFILES` (`0x8100`) feature driver.
//!
//! Controls host software mode vs. onboard profile memory mode, profile slot selection,
//! onboard memory sector reading, writing, and CRC verification for devices such as the G502 HERO.

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureId, FeatureTable, HidppFeature, check_feature_version};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_GET_DESCRIPTION: u8 = 0;
const FN_SET_ONBOARD_MODE: u8 = 1;
const FN_GET_ONBOARD_MODE: u8 = 2;
const FN_SET_CURRENT_PROFILE: u8 = 3;
const FN_GET_CURRENT_PROFILE: u8 = 4;
const FN_READ_MEMORY: u8 = 5;
const FN_WRITE_ADDR: u8 = 6;
const FN_WRITE_DATA: u8 = 7;
const FN_WRITE_END: u8 = 8;

/// Operational mode of the device: onboard profile memory vs. host software mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnboardMode {
    /// Onboard profile memory mode (`0x01`).
    /// Device executes macros, DPI profiles, and LED settings directly from internal flash.
    /// In this mode, volatile host writes (e.g. `0x8060 setReportRate`) return `0x02 InvalidArgument`.
    Onboard,
    /// Host / software mode (`0x02`).
    /// Device delegates polling rate and volatile lighting control to host software without requiring keepalives.
    Host,
    /// Unrecognized mode code.
    Other(u8),
}

impl From<u8> for OnboardMode {
    fn from(val: u8) -> Self {
        match val {
            0x01 => Self::Onboard,
            0x02 => Self::Host,
            other => Self::Other(other),
        }
    }
}

impl From<OnboardMode> for u8 {
    fn from(mode: OnboardMode) -> Self {
        match mode {
            OnboardMode::Onboard => 0x01,
            OnboardMode::Host => 0x02,
            OnboardMode::Other(val) => val,
        }
    }
}

/// Description of device onboard memory geometry and capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardMemoryDescription {
    /// Memory model / format identifier (e.g. `0x01`).
    pub memory_model: u8,
    /// Profile format layout identifier (e.g. `0x02` for G303/G502).
    pub profile_format: u8,
    /// Macro format identifier (e.g. `0x01`).
    pub macro_format: u8,
    /// Number of onboard profile slots (e.g. `5`).
    pub profile_count: u8,
    /// Number of out-of-box / ROM profile slots (e.g. `5`).
    pub rom_profile_count: u8,
    /// Number of physical buttons mapped in profiles (e.g. `11`).
    pub button_count: u8,
    /// Number of memory sectors (e.g. `16`).
    pub sector_count: u8,
    /// Size of each memory sector in bytes (e.g. `255`).
    pub sector_size: u16,
    /// Mechanical layout flags.
    pub mechanical_layout: u8,
    /// Various info flags (e.g. `0x01` = corded).
    pub various_info: u8,
}

/// Entry in the profile directory (sector 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileDirectoryEntry {
    /// Memory address / sector index of the profile (e.g. `0x0001`..`0x0005`).
    pub sector_address: u16,
    /// Whether this profile slot is enabled.
    pub enabled: bool,
}

/// Parsed representation of an onboard profile sector (format 2 / G502).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardProfile {
    /// Slot index (1-based, e.g. 1..=5).
    pub slot: u16,
    /// Report rate in Hz (e.g. 1000, 500, 250, 125).
    pub report_rate_hz: u16,
    /// Default DPI stage index (0-based).
    pub default_dpi_stage: u8,
    /// Switched / Shift (Sniper) DPI stage index (0-based).
    pub switched_dpi_stage: u8,
    /// Configured DPI stages (up to 5).
    pub dpi_stages: Vec<u16>,
    /// Profile display name (empty if default).
    pub name: String,
}

/// Computes the 16-bit CCITT CRC used for Logitech HID++ 2.0 onboard profile sectors.
///
/// Initial seed is `0xFFFF`, polynomial `0x1021`.
#[must_use]
pub fn calculate_sector_crc(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        let temp = (crc >> 8) ^ u16::from(byte);
        crc <<= 8;
        let mut quick = temp ^ (temp >> 4);
        crc ^= quick;
        quick = quick.wrapping_shl(5);
        crc ^= quick;
        quick = quick.wrapping_shl(7);
        crc ^= quick;
    }
    crc
}

/// Parses sector 0 into directory entries.
///
/// # Errors
///
/// Returns [`ProtocolError`] if sector data is truncated or corrupt.
pub fn parse_profile_directory(
    data: &[u8],
    profile_count: u8,
) -> Result<Vec<ProfileDirectoryEntry>, ProtocolError> {
    if data.len() < 4 * usize::from(profile_count) {
        return Err(ProtocolError::UnexpectedResponse {
            detail: format!("directory sector too short: {} bytes", data.len()),
        });
    }
    let mut entries = Vec::with_capacity(usize::from(profile_count));
    for i in 0..usize::from(profile_count) {
        let offset = i * 4;
        let addr = u16::from_be_bytes([data[offset], data[offset + 1]]);
        if addr == 0xFFFF {
            break;
        }
        let enabled = data[offset + 2] != 0;
        entries.push(ProfileDirectoryEntry {
            sector_address: addr,
            enabled,
        });
    }
    Ok(entries)
}

/// Parses an onboard profile sector (format 2 / G502).
///
/// # Errors
///
/// Returns [`ProtocolError`] if sector data is too short or malformed.
pub fn parse_onboard_profile(slot: u16, data: &[u8]) -> Result<OnboardProfile, ProtocolError> {
    if data.len() < 32 {
        return Err(ProtocolError::UnexpectedResponse {
            detail: format!("profile sector too short: {} bytes", data.len()),
        });
    }

    let report_rate_byte = data[0];
    let report_rate_hz = match report_rate_byte {
        1 => 1000,
        2 => 500,
        4 => 250,
        8 => 125,
        other => {
            if other == 0 {
                1000
            } else {
                1000 / u16::from(other)
            }
        }
    };

    let default_dpi_stage = data[1];
    let switched_dpi_stage = data[2];

    let mut dpi_stages = Vec::new();
    for i in 0..5 {
        let offset = 3 + 2 * i;
        if offset + 1 < data.len() {
            let dpi = u16::from_le_bytes([data[offset], data[offset + 1]]);
            if dpi > 0 && dpi != 0xFFFF {
                dpi_stages.push(dpi);
            }
        }
    }

    let mut name = String::new();
    if data.len() >= 208 {
        let name_bytes = &data[160..208];
        if !name_bytes.iter().all(|&b| b == 0xFF) {
            let trimmed = name_bytes.split(|&b| b == 0).next().unwrap_or(name_bytes);
            name = String::from_utf8_lossy(trimmed).trim().to_string();
        }
    }

    Ok(OnboardProfile {
        slot,
        report_rate_hz,
        default_dpi_stage,
        switched_dpi_stage,
        dpi_stages,
        name,
    })
}

/// Driver for `ONBOARD_PROFILES` (`0x8100`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnboardProfilesDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for OnboardProfilesDriver {
    const FEATURE_ID: FeatureId = FeatureId::ONBOARD_PROFILES;
    const MAX_SUPPORTED_VERSION: u8 = 0;
}

impl OnboardProfilesDriver {
    /// Creates a driver instance for a verified `FeatureEntry`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if the entry feature ID is not `0x8100`.
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected ONBOARD_PROFILES (0x8100), got {}", entry.id),
            });
        }
        let _supported =
            check_feature_version(Self::FEATURE_ID, entry.version, Self::MAX_SUPPORTED_VERSION);
        Ok(Self {
            feature_index: entry.index,
            version: entry.version,
        })
    }

    /// Discovers the driver from the session's feature table.
    #[must_use]
    pub fn from_table(table: &FeatureTable) -> Option<Self> {
        table
            .get(Self::FEATURE_ID)
            .and_then(|e| Self::from_entry(e).ok())
    }

    /// Returns the resolved feature index.
    #[must_use]
    pub const fn feature_index(&self) -> FeatureIndex {
        self.feature_index
    }

    /// Returns the runtime feature version reported by firmware.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// Queries the onboard memory layout and capabilities (`fn 0 getDescription`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_description<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<OnboardMemoryDescription, HidppError> {
        let function = FunctionId::new(FN_GET_DESCRIPTION)?;
        let reply = session.request(device_index, self.feature_index, function, &[])?;
        let params = reply.params();
        if params.len() < 11 {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "ONBOARD_PROFILES getDescription returned {} bytes (expected >= 11)",
                    params.len()
                ),
            }));
        }
        let sector_size = u16::from_be_bytes([params[7], params[8]]);
        Ok(OnboardMemoryDescription {
            memory_model: params[0],
            profile_format: params[1],
            macro_format: params[2],
            profile_count: params[3],
            rom_profile_count: params[4],
            button_count: params[5],
            sector_count: params[6],
            sector_size,
            mechanical_layout: params[9],
            various_info: params[10],
        })
    }

    /// Queries the current operating mode (`fn 2 getOnboardMode`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_onboard_mode<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<OnboardMode, HidppError> {
        let function = FunctionId::new(FN_GET_ONBOARD_MODE)?;
        let reply = session.request(device_index, self.feature_index, function, &[])?;
        let params = reply.params();
        if params.is_empty() {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: "ONBOARD_PROFILES getOnboardMode returned empty parameters".into(),
            }));
        }
        Ok(OnboardMode::from(params[0]))
    }

    /// Switches the operating mode (`fn 1 setOnboardMode`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails.
    pub fn set_onboard_mode<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        mode: OnboardMode,
    ) -> Result<(), HidppError> {
        let mode_byte = u8::from(mode);
        let function = FunctionId::new(FN_SET_ONBOARD_MODE)?;
        let _ = session.request(device_index, self.feature_index, function, &[mode_byte])?;
        Ok(())
    }

    /// Switches the active onboard profile slot (`fn 3 setCurrentProfile`).
    ///
    /// Slot numbers are 1-based (e.g. `1..=5`). Setting slot `0` will be rejected by firmware.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if `slot == 0`, or an error if transport fails.
    pub fn set_current_profile<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        slot: u16,
    ) -> Result<(), HidppError> {
        if slot == 0 {
            return Err(HidppError::Protocol(ProtocolError::InvalidArgument {
                detail: "profile slot index must be >= 1 (slot 0 is directory sector)".into(),
            }));
        }
        let slot_bytes = slot.to_be_bytes();
        let function = FunctionId::new(FN_SET_CURRENT_PROFILE)?;
        let _ = session.request(device_index, self.feature_index, function, &slot_bytes)?;
        Ok(())
    }

    /// Queries the currently active onboard profile slot (`fn 4 getCurrentProfile`).
    ///
    /// Returns `0` when the device is operating in Host mode.
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or parameters are truncated.
    pub fn get_current_profile<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<u16, HidppError> {
        let function = FunctionId::new(FN_GET_CURRENT_PROFILE)?;
        let reply = session.request(device_index, self.feature_index, function, &[])?;
        let params = reply.params();
        if params.len() < 2 {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "ONBOARD_PROFILES getCurrentProfile returned {} bytes (expected >= 2)",
                    params.len()
                ),
            }));
        }
        Ok(u16::from_be_bytes([params[0], params[1]]))
    }

    /// Reads a memory sector in 16-byte chunks (`fn 5`).
    ///
    /// Validates sector CRC in the last 2 bytes when `verify_crc` is true.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] if reading fails, or [`ProtocolError::UnexpectedResponse`] if CRC validation fails.
    pub fn read_sector<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        sector: u16,
        sector_size: u16,
        verify_crc: bool,
    ) -> Result<Vec<u8>, HidppError> {
        let function = FunctionId::new(FN_READ_MEMORY)?;
        let mut buffer = vec![0u8; usize::from(sector_size)];
        let sector_usize = usize::from(sector_size);

        let mut offset = 0;
        while offset < sector_usize {
            let read_offset = if sector_usize.saturating_sub(offset) < 16 {
                sector_usize.saturating_sub(16)
            } else {
                offset
            };

            let mut req = [0u8; 4];
            req[0..2].copy_from_slice(&sector.to_be_bytes());
            req[2..4].copy_from_slice(&(read_offset as u16).to_be_bytes());

            let reply = session.request(device_index, self.feature_index, function, &req)?;
            let chunk = reply.params();
            if chunk.len() < 16 {
                return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                    detail: format!("memory read returned {} bytes (expected 16)", chunk.len()),
                }));
            }

            let copy_len = (sector_usize - read_offset).min(16);
            let target_slice = &mut buffer[read_offset..read_offset + copy_len];
            target_slice.copy_from_slice(&chunk[..copy_len]);

            if read_offset == sector_usize.saturating_sub(16) {
                break;
            }
            offset += 16;
        }

        if verify_crc && sector_usize >= 2 {
            let data_len = sector_usize - 2;
            let read_crc = u16::from_be_bytes([buffer[data_len], buffer[data_len + 1]]);
            let calc_crc = calculate_sector_crc(&buffer[..data_len]);
            if read_crc != calc_crc {
                return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                    detail: format!(
                        "sector 0x{sector:04x} CRC mismatch: read 0x{read_crc:04x}, calculated 0x{calc_crc:04x}"
                    ),
                }));
            }
        }

        Ok(buffer)
    }

    /// Writes a memory sector in 16-byte chunks with start/data/end sequence (`fn 6`, `fn 7`, `fn 8`).
    ///
    /// Automatically computes and attaches the 16-bit CRC in the final 2 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] if writing fails.
    pub fn write_sector<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        sector: u16,
        data: &[u8],
    ) -> Result<(), HidppError> {
        let sector_size = data.len();
        if !(2..=512).contains(&sector_size) {
            return Err(HidppError::Protocol(ProtocolError::InvalidArgument {
                detail: format!("invalid sector size for write: {sector_size}"),
            }));
        }

        let mut payload = data.to_vec();
        let crc = calculate_sector_crc(&payload[..sector_size - 2]);
        payload[sector_size - 2..sector_size].copy_from_slice(&crc.to_be_bytes());

        // 1. write_start (fn 6): sector, sub_address=0, count=sector_size
        let fn_start = FunctionId::new(FN_WRITE_ADDR)?;
        let mut start_params = [0u8; 6];
        start_params[0..2].copy_from_slice(&sector.to_be_bytes());
        start_params[2..4].copy_from_slice(&0u16.to_be_bytes());
        start_params[4..6].copy_from_slice(&(sector_size as u16).to_be_bytes());
        let _ = session.request(device_index, self.feature_index, fn_start, &start_params)?;

        // 2. write_data (fn 7): 16 bytes per chunk
        let fn_write = FunctionId::new(FN_WRITE_DATA)?;
        for chunk in payload.chunks(16) {
            let mut chunk_buf = [0u8; 16];
            chunk_buf[..chunk.len()].copy_from_slice(chunk);
            let _ = session.request(device_index, self.feature_index, fn_write, &chunk_buf)?;
        }

        // 3. write_end (fn 8)
        let fn_end = FunctionId::new(FN_WRITE_END)?;
        let _ = session.request(device_index, self.feature_index, fn_end, &[])?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::frame::{HidppFrame, ReportId, SwId};
    use crate::session::SessionConfig;
    use crate::transport::{HidDeviceInfo, MockTransport};

    fn test_session(mock: MockTransport) -> HidppSession<MockTransport> {
        let cfg = SessionConfig {
            response_timeout: Duration::from_millis(50),
            busy_initial_backoff: Duration::ZERO,
            ..SessionConfig::default()
        };
        HidppSession::with_config(mock, cfg)
    }

    #[test]
    fn golden_g502_hero_onboard_profiles_mode_switch_and_query() {
        let feat_idx = FeatureIndex(0x0C);
        let dev_idx = DeviceIndex::DIRECT;
        let entry = FeatureEntry {
            index: feat_idx,
            id: FeatureId::ONBOARD_PROFILES,
            version: 0,
            flags: crate::feature::FeatureFlags::default(),
        };
        let driver = OnboardProfilesDriver::from_entry(&entry).unwrap();
        assert_eq!(driver.version(), 0);

        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);

        // 1. fn 0 getDescription -> G502 HERO memory layout
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[
                    0x01, 0x02, 0x01, 0x05, 0x05, 0x0B, 0x10, 0x00, 0xFF, 0x0A, 0x01, 0x00, 0x00,
                    0x00, 0x00, 0x00,
                ],
            )
            .unwrap(),
        );

        // 2. fn 2 getOnboardMode -> initially Onboard (0x01)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x01, 0x00],
            )
            .unwrap(),
        );

        // 3. fn 1 setOnboardMode -> switch to Host mode (0x02)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00],
            )
            .unwrap(),
        );

        // 4. fn 2 getOnboardMode -> now Host (0x02)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x02, 0x00],
            )
            .unwrap(),
        );

        // 5. fn 4 getCurrentProfile -> in Host mode returns 0
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(4).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00],
            )
            .unwrap(),
        );

        // 6. fn 1 setOnboardMode -> switch back to Onboard mode (0x01)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00],
            )
            .unwrap(),
        );

        // 7. fn 4 getCurrentProfile -> returns Slot 1
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(4).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x01],
            )
            .unwrap(),
        );

        let mut session = test_session(mock);

        let desc = driver.get_description(&mut session, dev_idx).unwrap();
        assert_eq!(desc.memory_model, 0x01);
        assert_eq!(desc.profile_format, 0x02);
        assert_eq!(desc.profile_count, 5);
        assert_eq!(desc.button_count, 11);
        assert_eq!(desc.sector_count, 16);
        assert_eq!(desc.sector_size, 255);

        let mode = driver.get_onboard_mode(&mut session, dev_idx).unwrap();
        assert_eq!(mode, OnboardMode::Onboard);

        driver
            .set_onboard_mode(&mut session, dev_idx, OnboardMode::Host)
            .unwrap();

        let host_mode = driver.get_onboard_mode(&mut session, dev_idx).unwrap();
        assert_eq!(host_mode, OnboardMode::Host);

        let cur_prof_host = driver.get_current_profile(&mut session, dev_idx).unwrap();
        assert_eq!(cur_prof_host, 0);

        driver
            .set_onboard_mode(&mut session, dev_idx, OnboardMode::Onboard)
            .unwrap();

        let cur_prof_onboard = driver.get_current_profile(&mut session, dev_idx).unwrap();
        assert_eq!(cur_prof_onboard, 1);
    }

    #[test]
    fn set_current_profile_rejects_slot_zero() {
        let feat_idx = FeatureIndex(0x0C);
        let dev_idx = DeviceIndex::DIRECT;
        let entry = FeatureEntry {
            index: feat_idx,
            id: FeatureId::ONBOARD_PROFILES,
            version: 0,
            flags: crate::feature::FeatureFlags::default(),
        };
        let driver = OnboardProfilesDriver::from_entry(&entry).unwrap();
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mock = MockTransport::new(info);
        let mut session = test_session(mock);

        let res = driver.set_current_profile(&mut session, dev_idx, 0);
        assert!(res.is_err());
    }

    #[test]
    fn test_calculate_sector_crc_known_vector() {
        // Sector 0 vector from real G502 HERO
        let mut sector0 = vec![0xFF; 255];
        sector0[0..4].copy_from_slice(&[0x00, 0x01, 0x01, 0xFF]);
        sector0[4..8].copy_from_slice(&[0x00, 0x02, 0x01, 0xFF]);
        sector0[8..12].copy_from_slice(&[0x00, 0x03, 0x01, 0xFF]);
        sector0[12..16].copy_from_slice(&[0x00, 0x04, 0x01, 0xFF]);
        sector0[16..20].copy_from_slice(&[0x00, 0x05, 0x01, 0xFF]);

        let crc = calculate_sector_crc(&sector0[..253]);
        assert_eq!(crc, 0x8601);
    }

    #[test]
    fn test_parse_directory_and_profile() {
        let mut sector0 = vec![0xFF; 255];
        sector0[0..4].copy_from_slice(&[0x00, 0x01, 0x01, 0xFF]);
        sector0[4..8].copy_from_slice(&[0x00, 0x02, 0x00, 0xFF]);
        sector0[8..12].copy_from_slice(&[0x00, 0x03, 0x01, 0xFF]);
        sector0[12..16].copy_from_slice(&[0xFF, 0xFF, 0x00, 0xFF]);

        let dir = parse_profile_directory(&sector0, 5).unwrap();
        assert_eq!(dir.len(), 3);
        assert_eq!(dir[0].sector_address, 1);
        assert!(dir[0].enabled);
        assert_eq!(dir[1].sector_address, 2);
        assert!(!dir[1].enabled);
        assert_eq!(dir[2].sector_address, 3);
        assert!(dir[2].enabled);

        let mut sector1 = vec![0xFF; 255];
        sector1[0] = 1; // 1000 Hz
        sector1[1] = 1; // default stage index 1
        sector1[2] = 0; // sniper stage index 0
        sector1[3..5].copy_from_slice(&1200u16.to_le_bytes());
        sector1[5..7].copy_from_slice(&2400u16.to_le_bytes());
        sector1[7..9].copy_from_slice(&3200u16.to_le_bytes());
        sector1[9..11].copy_from_slice(&0u16.to_le_bytes());

        let prof = parse_onboard_profile(1, &sector1).unwrap();
        assert_eq!(prof.slot, 1);
        assert_eq!(prof.report_rate_hz, 1000);
        assert_eq!(prof.default_dpi_stage, 1);
        assert_eq!(prof.switched_dpi_stage, 0);
        assert_eq!(prof.dpi_stages, vec![1200, 2400, 3200]);
        assert_eq!(prof.name, "");
    }

    #[test]
    fn test_read_sector_crc_mismatch_detected() {
        let feat_idx = FeatureIndex(0x0C);
        let dev_idx = DeviceIndex::DIRECT;
        let entry = FeatureEntry {
            index: feat_idx,
            id: FeatureId::ONBOARD_PROFILES,
            version: 0,
            flags: crate::feature::FeatureFlags::default(),
        };
        let driver = OnboardProfilesDriver::from_entry(&entry).unwrap();
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);

        // Push 16 chunks of 16 bytes for a 255-byte sector
        let fn5 = FunctionId::new(5).unwrap();
        for _ in 0..16 {
            mock.push_frame(
                HidppFrame::new(
                    ReportId::Long,
                    dev_idx,
                    feat_idx,
                    fn5,
                    SwId::DEFAULT_REQUEST,
                    &[0xAA; 16],
                )
                .unwrap(),
            );
        }

        let mut session = test_session(mock);
        // Sector CRC will not match 0xAAAA
        let res = driver.read_sector(&mut session, dev_idx, 1, 255, true);
        assert!(res.is_err());
        let err_str = format!("{:?}", res.unwrap_err());
        assert!(err_str.contains("CRC mismatch"));
    }
}
