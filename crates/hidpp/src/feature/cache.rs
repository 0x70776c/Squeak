//! Runtime feature table discovery and invalidatable per-device cache.

use std::collections::HashMap;

use tracing::{debug, warn};

use crate::error::{HidppError, ProtocolError};
use crate::feature::feature_set::IFeatureSet;
use crate::feature::root::IRoot;
use crate::feature::{
    FeatureEntry, FeatureFlags, FeatureId, HidppFeature, ProtocolVersion, check_feature_version,
};
use crate::frame::{DeviceIndex, FeatureIndex};
use crate::session::HidppSession;
use crate::transport::HidTransport;

const DEFAULT_PING_DATA: u8 = 0x53;

/// Runtime-discovered HID++ 2.0 feature table for a single connected device endpoint/slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureTable {
    device_index: DeviceIndex,
    protocol_version: ProtocolVersion,
    entries: Vec<FeatureEntry>,
}

impl FeatureTable {
    /// Constructs a [`FeatureTable`] from pre-discovered entries (useful for tests/snapshots).
    #[must_use]
    pub fn from_entries(
        device_index: DeviceIndex,
        protocol_version: ProtocolVersion,
        entries: Vec<FeatureEntry>,
    ) -> Self {
        Self {
            device_index,
            protocol_version,
            entries,
        }
    }

    /// Probes protocol version and enumerates all features via `IRoot` (`0x0000`) and
    /// `IFeatureSet` (`0x0001`).
    ///
    /// Feature indices are always resolved dynamically at runtime (`IRoot::getFeature(0x0001)`
    /// followed by `IFeatureSet::getCount` and `getFeatureID(1..=count)`), never hardcoded.
    ///
    /// # Errors
    ///
    /// Returns [`HidppError`] if `IRoot::ping` fails (including HID++ 1.0 legacy detection),
    /// transport fails, or feature enumeration returns an invalid response.
    pub fn discover<T: HidTransport>(
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<Self, HidppError> {
        let protocol_version = IRoot::ping(session, device_index, DEFAULT_PING_DATA)?;

        let mut entries = vec![FeatureEntry {
            index: FeatureIndex::IROOT,
            id: FeatureId::IROOT,
            version: 0,
            flags: FeatureFlags::default(),
        }];

        let Some(fs_resolved) = IRoot::get_feature(session, device_index, FeatureId::IFEATURE_SET)?
        else {
            warn!(
                device_index = %device_index,
                %protocol_version,
                "device reports HID++ 2.0+ but lacks IFeatureSet (0x0001); using minimal IRoot table"
            );
            return Ok(Self {
                device_index,
                protocol_version,
                entries,
            });
        };

        let _version_ok = check_feature_version(
            FeatureId::IFEATURE_SET,
            fs_resolved.version,
            IFeatureSet::MAX_SUPPORTED_VERSION,
        );

        let count = IFeatureSet::get_count(session, device_index, fs_resolved.index)?;
        debug!(
            device_index = %device_index,
            %protocol_version,
            feature_set_index = %fs_resolved.index,
            count,
            "discovered HID++ 2.0 feature count"
        );

        entries.reserve(usize::from(count));
        for idx in 1..=count {
            let entry = IFeatureSet::get_feature_id(session, device_index, fs_resolved.index, idx)?;
            entries.push(entry);
        }

        Ok(Self {
            device_index,
            protocol_version,
            entries,
        })
    }

    /// Returns the addressed [`DeviceIndex`].
    #[must_use]
    pub const fn device_index(&self) -> DeviceIndex {
        self.device_index
    }

    /// Returns the HID++ protocol version returned by `IRoot::ping`.
    #[must_use]
    pub const fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    /// Returns all discovered feature entries in table order (including `IRoot` at `0x00`).
    #[must_use]
    pub fn entries(&self) -> &[FeatureEntry] {
        &self.entries
    }

    /// Looks up an entry by 16-bit [`FeatureId`].
    #[must_use]
    pub fn get(&self, id: FeatureId) -> Option<&FeatureEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// Returns the runtime [`FeatureIndex`] for `id`, or `None` if unsupported.
    #[must_use]
    pub fn get_index(&self, id: FeatureId) -> Option<FeatureIndex> {
        self.get(id).map(|entry| entry.index)
    }

    /// Looks up a [`FeatureEntry`] by its runtime [`FeatureIndex`].
    #[must_use]
    pub fn by_index(&self, index: FeatureIndex) -> Option<&FeatureEntry> {
        self.entries.iter().find(|entry| entry.index == index)
    }

    /// Returns `true` if `id` is present in this table.
    #[must_use]
    pub fn contains(&self, id: FeatureId) -> bool {
        self.get(id).is_some()
    }

    /// Resolves a required [`FeatureEntry`], returning [`ProtocolError::FeatureNotSupported`]
    /// when absent.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::FeatureNotSupported`] if the device does not advertise `id`.
    pub fn require(&self, id: FeatureId) -> Result<&FeatureEntry, ProtocolError> {
        self.get(id)
            .ok_or(ProtocolError::FeatureNotSupported { feature_id: id })
    }
}

/// Cache of runtime [`FeatureTable`]s keyed by [`DeviceIndex`], invalidated on reconnect or
/// firmware change.
#[derive(Debug, Default)]
pub struct FeatureCache {
    tables: HashMap<DeviceIndex, FeatureTable>,
}

impl FeatureCache {
    /// Creates an empty feature table cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a cached [`FeatureTable`] for `device_index` if present.
    #[must_use]
    pub fn get(&self, device_index: DeviceIndex) -> Option<&FeatureTable> {
        self.tables.get(&device_index)
    }

    /// Inserts or replaces the cached [`FeatureTable`] for its device index.
    pub fn insert(&mut self, table: FeatureTable) {
        self.tables.insert(table.device_index, table);
    }

    /// Invalidates the cached feature table for `device_index` (e.g., on slot reconnect).
    pub fn invalidate_device(&mut self, device_index: DeviceIndex) -> Option<FeatureTable> {
        self.tables.remove(&device_index)
    }

    /// Clears all cached feature tables (e.g., after transport disconnect/reset).
    pub fn invalidate_all(&mut self) {
        self.tables.clear();
    }

    /// Returns a cached [`FeatureTable`] for `device_index`, or discovers and caches it on miss.
    ///
    /// # Errors
    ///
    /// Propagates any [`HidppError`] returned by [`FeatureTable::discover`].
    pub fn get_or_discover<T: HidTransport>(
        &mut self,
        session: &mut HidppSession<T>,
        device_index: DeviceIndex,
    ) -> Result<&FeatureTable, HidppError> {
        match self.tables.entry(device_index) {
            std::collections::hash_map::Entry::Occupied(entry) => Ok(entry.into_mut()),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let table = FeatureTable::discover(session, device_index)?;
                Ok(entry.insert(table))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::error::Hidpp10ErrorCode;
    use crate::frame::{HidppFrame, ReportId, SwId};
    use crate::session::SessionConfig;
    use crate::transport::{HidDeviceInfo, MockTransport};

    fn test_config() -> SessionConfig {
        SessionConfig {
            sw_id: SwId::DEFAULT_REQUEST,
            response_timeout: Duration::from_millis(50),
            max_busy_retries: 1,
            busy_initial_backoff: Duration::ZERO,
        }
    }

    fn fn_id(id: u8) -> crate::frame::FunctionId {
        crate::frame::FunctionId::new(id).unwrap()
    }

    fn queue_discovery_sequence(mock: &mut MockTransport, fs_index: u8) {
        // 1. IRoot::ping(0x53) -> 4.5
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x04, 0x05, DEFAULT_PING_DATA],
            )
            .unwrap(),
        );
        // 2. IRoot::getFeature(0x0001) -> resolved to fs_index (testing non-hardcoded index!)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[fs_index, 0x00, 0x01],
            )
            .unwrap(),
        );
        // 3. IFeatureSet::getCount() -> 3 entries
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex(fs_index),
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[3, 0, 0],
            )
            .unwrap(),
        );
        // 4. getFeatureID(1) -> IFeatureSet at fs_index, or DEVICE_NAME if fs_index == 2
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(fs_index),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x05, 0x00, 0x00],
            )
            .unwrap(),
        );
        // 5. getFeatureID(2) -> IFeatureSet itself + version 1
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(fs_index),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x01, 0x00, 0x01],
            )
            .unwrap(),
        );
        // 6. getFeatureID(3) -> ADJUSTABLE_DPI (0x2201) + unknown future version 7
        mock.push_frame(
            HidppFrame::new(
                ReportId::Long,
                DeviceIndex::DIRECT,
                FeatureIndex(fs_index),
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x22, 0x01, 0x00, 0x07],
            )
            .unwrap(),
        );
    }

    #[test]
    fn discovers_features_dynamically_without_hardcoded_indices_and_caches() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "Test Mouse");
        let mut mock = MockTransport::new(info);
        // Put IFeatureSet at index 0x02 instead of 0x01 to verify dynamic lookup!
        queue_discovery_sequence(&mut mock, 0x02);

        let mut session = HidppSession::with_config(mock, test_config());
        let mut cache = FeatureCache::new();

        {
            let table = cache
                .get_or_discover(&mut session, DeviceIndex::DIRECT)
                .unwrap();
            assert_eq!(
                table.protocol_version(),
                ProtocolVersion { major: 4, minor: 5 }
            );
            assert_eq!(table.entries().len(), 4); // IRoot + 3 features
            assert_eq!(table.get_index(FeatureId::IROOT), Some(FeatureIndex::IROOT));
            assert_eq!(
                table.get_index(FeatureId::DEVICE_NAME),
                Some(FeatureIndex(1))
            );
            assert_eq!(
                table.get_index(FeatureId::IFEATURE_SET),
                Some(FeatureIndex(2))
            );
            assert_eq!(
                table.get_index(FeatureId::ADJUSTABLE_DPI),
                Some(FeatureIndex(3))
            );
            assert_eq!(table.get(FeatureId::ADJUSTABLE_DPI).unwrap().version, 7);
            assert!(matches!(
                table.require(FeatureId::RGB_EFFECTS),
                Err(ProtocolError::FeatureNotSupported {
                    feature_id: FeatureId::RGB_EFFECTS
                })
            ));
        }

        // Verify all getCount/getFeatureID requests addressed runtime index 0x02
        let writes_before = session.transport().written_frames().len();
        assert_eq!(writes_before, 6);
        assert_eq!(
            session.transport().written_frames()[2].feature_index(),
            FeatureIndex(0x02)
        );

        // Cache hit does not issue additional I/O
        let _cached = cache
            .get_or_discover(&mut session, DeviceIndex::DIRECT)
            .unwrap();
        assert_eq!(session.transport().written_frames().len(), writes_before);

        // Invalidate + re-discover issues fresh discovery frames
        assert!(cache.invalidate_device(DeviceIndex::DIRECT).is_some());
        queue_discovery_sequence(session.transport_mut(), 0x02);
        let _rediscovered = cache
            .get_or_discover(&mut session, DeviceIndex::DIRECT)
            .unwrap();
        assert_eq!(session.transport().written_frames().len(), 12);
    }

    #[test]
    fn degrades_gracefully_when_ifeature_set_missing_and_rejects_legacy_10() {
        let info = HidDeviceInfo::mock(0x046D, 0xC08B, "Bootloader Device");
        let mut mock = MockTransport::new(info);
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(1),
                SwId::DEFAULT_REQUEST,
                &[0x04, 0x00, DEFAULT_PING_DATA],
            )
            .unwrap(),
        );
        // getFeature(0x0001) returns index 0x00 (absent)
        mock.push_frame(
            HidppFrame::new(
                ReportId::Short,
                DeviceIndex::DIRECT,
                FeatureIndex::IROOT,
                fn_id(0),
                SwId::DEFAULT_REQUEST,
                &[0x00, 0x00, 0x00],
            )
            .unwrap(),
        );

        let mut session = HidppSession::with_config(mock, test_config());
        let table = FeatureTable::discover(&mut session, DeviceIndex::DIRECT).unwrap();
        assert_eq!(table.entries().len(), 1);
        assert!(table.contains(FeatureId::IROOT));
        assert!(!table.contains(FeatureId::IFEATURE_SET));

        // Now simulate legacy HID++ 1.0 response
        let legacy_info = HidDeviceInfo::mock(0x046D, 0xC52B, "Legacy Device");
        let mut legacy_mock = MockTransport::new(legacy_info);
        legacy_mock
            .push_frame(HidppFrame::decode(&[0x10, 0xFF, 0x8F, 0x00, 0x1A, 0x01, 0x00]).unwrap());
        let mut legacy_session = HidppSession::with_config(legacy_mock, test_config());
        let err = FeatureTable::discover(&mut legacy_session, DeviceIndex::DIRECT).unwrap_err();
        assert!(matches!(
            err,
            HidppError::Protocol(ProtocolError::LegacyDeviceNotSupported {
                error_code: Hidpp10ErrorCode::InvalidSubId
            })
        ));
    }
}
