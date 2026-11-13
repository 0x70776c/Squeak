//! `COLOR_LED_EFFECTS` (`0x8070`) feature driver.
//!
//! Provides volatile per-zone LED configuration (Off, Static, Breathing, Color Cycle)
//! for HID++ 2.0 gaming mice such as the G502 HERO.

use core::fmt;

use crate::error::{HidppError, ProtocolError};
use crate::feature::{FeatureEntry, FeatureId, FeatureTable, HidppFeature, check_feature_version};
use crate::frame::{DeviceIndex, FeatureIndex, FunctionId};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const FN_GET_INFO: u8 = 0;
const FN_GET_ZONE_INFO: u8 = 1;
const FN_GET_EFFECT_INFO: u8 = 2;
const FN_SET_LED_EFFECT: u8 = 3;

/// Symbolic effect type identifier reported by `getEffectInfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LedEffectId {
    /// Effect 0x0000: LED is completely turned off.
    Off,
    /// Effect 0x0001: Fixed static solid color.
    Static,
    /// Effect 0x000A: Breathing/pulsing single color.
    Breathing,
    /// Effect 0x0003: Continuous RGB color spectrum cycle.
    ColorCycle,
    /// Other vendor effect ID.
    Other(u16),
}

impl From<u16> for LedEffectId {
    fn from(raw: u16) -> Self {
        match raw {
            0x0000 => Self::Off,
            0x0001 => Self::Static,
            0x000A => Self::Breathing,
            0x0003 => Self::ColorCycle,
            other => Self::Other(other),
        }
    }
}

impl From<LedEffectId> for u16 {
    fn from(eff: LedEffectId) -> Self {
        match eff {
            LedEffectId::Off => 0x0000,
            LedEffectId::Static => 0x0001,
            LedEffectId::Breathing => 0x000A,
            LedEffectId::ColorCycle => 0x0003,
            LedEffectId::Other(raw) => raw,
        }
    }
}

/// 24-bit RGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RgbColor {
    /// Red component (`0..=255`).
    pub r: u8,
    /// Green component (`0..=255`).
    pub g: u8,
    /// Blue component (`0..=255`).
    pub b: u8,
}

impl RgbColor {
    /// Creates a new RGB color.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Converts to standard 24-bit hex integer `0x00RRGGBB`.
    #[must_use]
    pub const fn to_hex(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }
}

impl fmt::Display for RgbColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

/// Volatile LED effect configuration for a zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedEffect {
    /// Off: LED completely turned off.
    Off,
    /// Static: fixed RGB color.
    Static {
        /// 24-bit RGB color.
        color: RgbColor,
    },
    /// Breathing: pulsing single RGB color with period in milliseconds.
    Breathing {
        /// Pulsing color.
        color: RgbColor,
        /// Cycle period in milliseconds (e.g. 2000 ms).
        period_ms: u16,
        /// Brightness percentage (`0..=100`).
        brightness_pct: u8,
    },
    /// Color Cycle: cycling spectrum with period in milliseconds.
    ColorCycle {
        /// Full cycle duration in milliseconds (e.g. 4000 ms).
        period_ms: u16,
        /// Brightness percentage (`0..=100`).
        brightness_pct: u8,
    },
}

impl LedEffect {
    /// Serializes this effect into payload bytes for `fn 3 setLedEffect`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if parameters (like brightness > 100) are invalid.
    pub fn to_payload(self, zone_index: u8) -> Result<[u8; 16], ProtocolError> {
        let mut buf = [0u8; 16];
        buf[0] = zone_index;
        match self {
            Self::Off => {
                buf[1] = 0x00; // Effect index 0 = Off
            }
            Self::Static { color } => {
                buf[1] = 0x01; // Effect index 1 = Static
                buf[2] = color.r;
                buf[3] = color.g;
                buf[4] = color.b;
                buf[5] = 0x02; // Static effect mode marker
            }
            Self::Breathing {
                color,
                period_ms,
                brightness_pct,
            } => {
                if brightness_pct > 100 {
                    return Err(ProtocolError::InvalidArgument {
                        detail: format!("brightness percentage {brightness_pct} exceeds 100%"),
                    });
                }
                buf[1] = 0x02; // Effect index 2 = Breathing
                buf[2] = color.r;
                buf[3] = color.g;
                buf[4] = color.b;
                let period_bytes = period_ms.to_be_bytes();
                buf[5] = period_bytes[0];
                buf[6] = period_bytes[1];
                buf[7] = 0x00;
                buf[8] = brightness_pct;
            }
            Self::ColorCycle {
                period_ms,
                brightness_pct,
            } => {
                if brightness_pct > 100 {
                    return Err(ProtocolError::InvalidArgument {
                        detail: format!("brightness percentage {brightness_pct} exceeds 100%"),
                    });
                }
                buf[1] = 0x03; // Effect index 3 = Color Cycle
                let period_bytes = period_ms.to_be_bytes();
                buf[7] = period_bytes[0];
                buf[8] = period_bytes[1];
                buf[9] = brightness_pct;
            }
        }
        Ok(buf)
    }
}

/// Metadata for an individual physical LED zone on the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedZoneInfo {
    /// Zero-based zone index (e.g. 0 for Logo, 1 for DPI LEDs).
    pub zone_index: u8,
    /// Physical location code reported by firmware (e.g. 0x0001 = Primary/Logo, 0x0002 = Secondary/DPI).
    pub location: u16,
    /// Number of supported effects reported for this zone.
    pub effect_count: u8,
    /// List of effect IDs supported by this zone.
    pub supported_effects: Vec<LedEffectId>,
}

/// Driver for `COLOR_LED_EFFECTS` (`0x8070`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorLedEffectsDriver {
    feature_index: FeatureIndex,
    version: u8,
}

impl HidppFeature for ColorLedEffectsDriver {
    const FEATURE_ID: FeatureId = FeatureId::COLOR_LED_EFFECTS;
    const MAX_SUPPORTED_VERSION: u8 = 3;
}

impl ColorLedEffectsDriver {
    /// Creates a driver instance for a verified `FeatureEntry`.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidArgument`] if the entry feature ID is not `0x8070`.
    pub fn from_entry(entry: &FeatureEntry) -> Result<Self, ProtocolError> {
        if entry.id != Self::FEATURE_ID {
            return Err(ProtocolError::InvalidArgument {
                detail: format!("expected COLOR_LED_EFFECTS (0x8070), got {}", entry.id),
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

    /// Queries the total number of LED zones supported by the device (`fn 0 getInfo`).
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails or the response frame is malformed.
    pub fn get_zone_count<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<u8, HidppError> {
        let function = FunctionId::new(FN_GET_INFO)?;
        let reply = session.request(device_index, self.feature_index, function, &[])?;
        let params = reply.params();
        if params.is_empty() {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: "COLOR_LED_EFFECTS getInfo returned empty parameters".into(),
            }));
        }
        Ok(params[0])
    }

    /// Queries information about a specific LED zone (`fn 1 getZoneInfo`).
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or response parameters are truncated.
    pub fn get_zone_info<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        zone_index: u8,
    ) -> Result<LedZoneInfo, HidppError> {
        let function = FunctionId::new(FN_GET_ZONE_INFO)?;
        let reply = session.request(device_index, self.feature_index, function, &[zone_index])?;
        let params = reply.params();
        if params.len() < 4 {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "COLOR_LED_EFFECTS getZoneInfo returned {} bytes (expected >= 4)",
                    params.len()
                ),
            }));
        }
        let reported_zone = params[0];
        if reported_zone != zone_index {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "getZoneInfo zone mismatch: requested {zone_index}, got {reported_zone}"
                ),
            }));
        }
        let location = u16::from_be_bytes([params[1], params[2]]);
        let effect_count = params[3];

        let mut supported_effects = Vec::with_capacity(effect_count as usize);
        for eff_idx in 0..effect_count {
            let eff_id = self.get_effect_info(session, device_index, zone_index, eff_idx)?;
            supported_effects.push(eff_id);
        }

        Ok(LedZoneInfo {
            zone_index,
            location,
            effect_count,
            supported_effects,
        })
    }

    /// Queries the effect identifier for a given effect index on a zone (`fn 2 getEffectInfo`).
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or the response frame is truncated.
    pub fn get_effect_info<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        zone_index: u8,
        effect_index: u8,
    ) -> Result<LedEffectId, HidppError> {
        let function = FunctionId::new(FN_GET_EFFECT_INFO)?;
        let reply = session.request(
            device_index,
            self.feature_index,
            function,
            &[zone_index, effect_index],
        )?;
        let params = reply.params();
        if params.len() < 4 {
            return Err(HidppError::Protocol(ProtocolError::UnexpectedResponse {
                detail: format!(
                    "COLOR_LED_EFFECTS getEffectInfo returned {} bytes (expected >= 4)",
                    params.len()
                ),
            }));
        }
        let raw_eff_id = u16::from_be_bytes([params[2], params[3]]);
        Ok(LedEffectId::from(raw_eff_id))
    }

    /// Sets a volatile LED effect for a zone (`fn 3 setLedEffect`).
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or parameters are invalid.
    pub fn set_led_effect<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
        zone_index: u8,
        effect: LedEffect,
    ) -> Result<(), HidppError> {
        let payload = effect.to_payload(zone_index)?;
        let function = FunctionId::new(FN_SET_LED_EFFECT)?;
        let _ = session.request(device_index, self.feature_index, function, &payload)?;
        Ok(())
    }

    /// Enumerates all LED zones and their supported effect IDs on the device.
    ///
    /// # Errors
    ///
    /// Returns an error if transport fails during zone discovery.
    pub fn query_all_zones<T: HidTransport>(
        &self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<Vec<LedZoneInfo>, HidppError> {
        let count = self.get_zone_count(session, device_index)?;
        let mut zones = Vec::with_capacity(count as usize);
        for z in 0..count {
            zones.push(self.get_zone_info(session, device_index, z)?);
        }
        Ok(zones)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use proptest::prelude::*;

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
    fn golden_g502_hero_color_led_effects_discovery_and_effects() {
        let feat_idx = FeatureIndex(0x02);
        let dev_idx = DeviceIndex::DIRECT;
        let entry = FeatureEntry {
            index: feat_idx,
            id: FeatureId::COLOR_LED_EFFECTS,
            version: 3,
            flags: crate::feature::FeatureFlags::default(),
        };
        let driver = ColorLedEffectsDriver::from_entry(&entry).unwrap();
        assert_eq!(driver.version(), 3);

        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "G502 HERO");
        let mut mock = MockTransport::new(info);

        // 1. fn 0 getInfo -> 2 zones
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(0).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x02, 0x00, 0x03],
            )
            .unwrap(),
        );

        // 2. fn 1 getZoneInfo for zone 0 -> location 0x0001 (Logo), 4 effects
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(1).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00, 0x01, 0x04],
            )
            .unwrap(),
        );

        // 3. fn 2 getEffectInfo for zone 0, eff 0..3:
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00, 0x00, 0x00],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x01, 0x00, 0x01],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x02, 0x00, 0x0A],
            )
            .unwrap(),
        );
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(2).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x03, 0x00, 0x03],
            )
            .unwrap(),
        );

        // 4. fn 3 setLedEffect: Static Red (#FF0000) on Zone 0
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                dev_idx,
                feat_idx,
                FunctionId::new(3).unwrap(),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00],
            )
            .unwrap(),
        );

        let mut session = test_session(mock);

        let count = driver.get_zone_count(&mut session, dev_idx).unwrap();
        assert_eq!(count, 2);

        let zone0 = driver.get_zone_info(&mut session, dev_idx, 0).unwrap();
        assert_eq!(zone0.zone_index, 0);
        assert_eq!(zone0.location, 1);
        assert_eq!(zone0.effect_count, 4);
        assert_eq!(
            zone0.supported_effects,
            vec![
                LedEffectId::Off,
                LedEffectId::Static,
                LedEffectId::Breathing,
                LedEffectId::ColorCycle,
            ]
        );

        let effect = LedEffect::Static {
            color: RgbColor::new(0xFF, 0x00, 0x00),
        };
        driver
            .set_led_effect(&mut session, dev_idx, 0, effect)
            .unwrap();
    }

    #[test]
    fn led_effect_payload_construction_matches_spec() {
        let off = LedEffect::Off.to_payload(1).unwrap();
        assert_eq!(off[0], 1);
        assert_eq!(off[1], 0);

        let breathing = LedEffect::Breathing {
            color: RgbColor::new(0, 0, 0xFF),
            period_ms: 2000,
            brightness_pct: 100,
        }
        .to_payload(0)
        .unwrap();
        assert_eq!(breathing[0], 0);
        assert_eq!(breathing[1], 2);
        assert_eq!(&breathing[2..5], &[0, 0, 0xFF]);
        assert_eq!(&breathing[5..7], &2000u16.to_be_bytes());
        assert_eq!(breathing[8], 100);

        let cycle = LedEffect::ColorCycle {
            period_ms: 4000,
            brightness_pct: 50,
        }
        .to_payload(0)
        .unwrap();
        assert_eq!(cycle[0], 0);
        assert_eq!(cycle[1], 3);
        assert_eq!(&cycle[7..9], &4000u16.to_be_bytes());
        assert_eq!(cycle[9], 50);

        assert!(
            LedEffect::Breathing {
                color: RgbColor::default(),
                period_ms: 1000,
                brightness_pct: 101,
            }
            .to_payload(0)
            .is_err()
        );
    }

    proptest! {
        #[test]
        fn rgb_color_hex_roundtrip(r in 0u8..=255, g in 0u8..=255, b in 0u8..=255) {
            let color = RgbColor::new(r, g, b);
            let hex = color.to_hex();
            assert_eq!(((hex >> 16) & 0xFF) as u8, r);
            assert_eq!(((hex >> 8) & 0xFF) as u8, g);
            assert_eq!((hex & 0xFF) as u8, b);
            let hex_str = format!("{color}");
            assert_eq!(hex_str.len(), 7);
            assert!(hex_str.starts_with('#'));
        }
    }
}
