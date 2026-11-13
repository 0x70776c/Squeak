//! Data-driven device definition catalog (`data/devices/*.toml`), validation, and generic fallback.
//!
//! Device files supply presentation labels, SVG button mappings, and documented firmware quirks
//! only. Feature indices and device capabilities always come from runtime HID++ 2.0 discovery.

use core::fmt;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Embedded built-in device definition files compiled into `squeak-core`.
const BUILTIN_TOMLS: &[(&str, &str)] = &[(
    "g502-hero.toml",
    include_str!("../../../data/devices/g502-hero.toml"),
)];

/// High-level device category stored in `data/devices/*.toml`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceKind {
    /// Mouse / trackball.
    Mouse,
    /// Keyboard.
    Keyboard,
    /// Headset / audio peripheral.
    Headset,
    /// Wireless USB receiver (e.g., Unifying / Bolt).
    Receiver,
    /// Other or generic HID++ 2.0 peripheral.
    Other,
}

impl fmt::Display for DeviceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Mouse => "mouse",
            Self::Keyboard => "keyboard",
            Self::Headset => "headset",
            Self::Receiver => "receiver",
            Self::Other => "other",
        };
        f.write_str(label)
    }
}

/// UI presentation metadata for a single programmable button/control ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ButtonDefinition {
    /// Control ID (CID) reported by `REPROG_CONTROLS_V4` (`0x1B04`).
    pub cid: u16,
    /// Human-readable control label shown in CLI/GUI.
    pub label: String,
    /// Optional SVG element `id` highlighting this control on the device diagram.
    #[serde(default)]
    pub svg_id: Option<String>,
}

/// Proven firmware quirk flags documented in `data/devices/*.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeviceQuirks {
    /// Human-readable quirk references with issue links.
    pub notes: Vec<String>,
}

/// Validated device definition loaded from a single TOML file in `data/devices/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDefinition {
    /// Display model name (e.g., `"G502 HERO"`).
    pub name: String,
    /// USB/BT vendor ID (for example `0x046D`).
    pub vendor_id: u16,
    /// All known USB/BT product IDs mapped to this definition.
    pub product_ids: Vec<u16>,
    /// Device form factor.
    pub kind: DeviceKind,
    /// Optional relative SVG asset filename under `data/devices/`.
    #[serde(default)]
    pub svg: Option<String>,
    /// Button presentation mappings indexed by runtime CID.
    #[serde(default)]
    pub buttons: Vec<ButtonDefinition>,
    /// Documented firmware quirk overrides (empty by default).
    #[serde(default)]
    pub quirks: DeviceQuirks,
}

/// Fallback presentation metadata synthesized when `(vendor_id, product_id)` has no TOML entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenericDeviceProfile {
    /// Discovered vendor ID.
    pub vendor_id: u16,
    /// Discovered product ID.
    pub product_id: u16,
    /// Best available display name (device-reported HID name or `"Generic HID++ 2.0 Device"`).
    pub name: String,
    /// Generic category until runtime features refine capabilities.
    pub kind: DeviceKind,
}

/// Result of resolving `(vendor_id, product_id)` against a [`DeviceDatabase`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedDevice<'a> {
    /// Exact match found in `data/devices/*.toml`.
    Known(&'a DeviceDefinition),
    /// Unrecognized HID++ 2.0 device running in generic fallback mode.
    Generic,
}

impl<'a> ResolvedDevice<'a> {
    /// Returns the display name (matching definition name, HID name, or generic label).
    #[must_use]
    pub fn display_name(self, hid_name: Option<&str>) -> String {
        match self {
            Self::Known(def) => def.name.clone(),
            Self::Generic => hid_name
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("Generic HID++ 2.0 Device")
                .to_owned(),
        }
    }

    /// Returns the resolved [`DeviceKind`].
    #[must_use]
    pub const fn kind(self) -> DeviceKind {
        match self {
            Self::Known(def) => def.kind,
            Self::Generic => DeviceKind::Other,
        }
    }

    /// Returns `true` when matched against an explicit TOML entry.
    #[must_use]
    pub const fn is_known(self) -> bool {
        matches!(self, Self::Known(_))
    }
}

/// Errors encountered while loading or validating `data/devices/*.toml`.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum DeviceDataError {
    /// Reading the device directory, TOML file, or referenced SVG failed.
    #[error("failed reading {path}: {source}")]
    Io {
        /// File or directory path involved.
        path: PathBuf,
        /// Underlying OS I/O error.
        #[source]
        source: std::io::Error,
    },

    /// Parsing a device TOML file failed.
    #[error("invalid device TOML in {source_name}: {source}")]
    TomlParse {
        /// Source filename or path.
        source_name: String,
        /// Underlying `toml` error.
        #[source]
        source: toml::de::Error,
    },

    /// Structural or schema invariant was violated in a device TOML file.
    #[error("validation error in {source_name}: {reason}")]
    Validation {
        /// Source filename or path.
        source_name: String,
        /// Specific constraint violation.
        reason: String,
    },

    /// Duplicate `(vendor_id, product_id)` registered by two device entries.
    #[error(
        "duplicate VID:PID 0x{vendor_id:04X}:0x{product_id:04X} claimed by both {first_source:?} and {second_source:?}"
    )]
    DuplicateProductId {
        /// Conflicting vendor ID.
        vendor_id: u16,
        /// Conflicting product ID.
        product_id: u16,
        /// First source claiming this ID.
        first_source: String,
        /// Second source claiming this ID.
        second_source: String,
    },

    /// Duplicate `cid` defined within the same device's `[[buttons]]` list.
    #[error("duplicate button cid 0x{cid:04X} in {source_name}")]
    DuplicateButtonCid {
        /// Source filename or path.
        source_name: String,
        /// Duplicated control ID.
        cid: u16,
    },

    /// Duplicate `svg_id` referenced within the same device's `[[buttons]]` list.
    #[error("duplicate svg_id {svg_id:?} in {source_name}")]
    DuplicateButtonSvgId {
        /// Source filename or path.
        source_name: String,
        /// Duplicated SVG element ID.
        svg_id: String,
    },

    /// Referenced SVG asset does not exist next to the device TOML file.
    #[error("missing SVG asset {svg_path} referenced by {source_name}")]
    MissingSvgFile {
        /// Source filename or path.
        source_name: String,
        /// Expected SVG path.
        svg_path: PathBuf,
    },

    /// Button's `svg_id` was missing or absent from the referenced SVG document.
    #[error("button cid 0x{cid:04X} references missing or unknown svg_id {svg_id:?} in {svg_path}")]
    MissingSvgElementId {
        /// Offending button CID.
        cid: u16,
        /// Referenced (or empty) `svg_id`.
        svg_id: String,
        /// Path to the SVG file checked.
        svg_path: PathBuf,
    },
}

/// In-memory index of validated device definitions keyed by `(vendor_id, product_id)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDatabase {
    definitions: Vec<DeviceDefinition>,
    sources: Vec<String>,
    index: HashMap<(u16, u16), usize>,
}

impl DeviceDatabase {
    /// Loads the built-in device database compiled into Squeak.
    ///
    /// # Errors
    ///
    /// Returns [`DeviceDataError`] if any embedded definition fails schema or uniqueness validation.
    pub fn builtin() -> Result<Self, DeviceDataError> {
        let parsed = BUILTIN_TOMLS
            .iter()
            .map(|(name, raw)| ((*name).to_owned(), (*raw).to_owned()))
            .collect::<Vec<_>>();
        Self::from_sources(parsed, None)
    }

    /// Loads and validates every `*.toml` file inside `dir`.
    ///
    /// Also validates referenced SVG files and all `[[buttons]]` `svg_id` element IDs against
    /// those SVGs.
    ///
    /// # Errors
    ///
    /// Returns [`DeviceDataError`] on directory read failures, invalid TOML, duplicate
    /// `(vendor_id, product_id)` pairs, duplicate CIDs, or invalid SVG references.
    pub fn load_from_dir(dir: &Path) -> Result<Self, DeviceDataError> {
        let entries = fs::read_dir(dir).map_err(|source| DeviceDataError::Io {
            path: dir.to_path_buf(),
            source,
        })?;

        let mut toml_paths = Vec::new();
        for entry_res in entries {
            let entry = entry_res.map_err(|source| DeviceDataError::Io {
                path: dir.to_path_buf(),
                source,
            })?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                toml_paths.push(path);
            }
        }
        toml_paths.sort();

        let mut sources = Vec::with_capacity(toml_paths.len());
        for path in toml_paths {
            let content = fs::read_to_string(&path).map_err(|source| DeviceDataError::Io {
                path: path.clone(),
                source,
            })?;
            sources.push((path.display().to_string(), content));
        }

        Self::from_sources(sources, Some(dir))
    }

    fn from_sources(
        entries: Vec<(String, String)>,
        svg_base_dir: Option<&Path>,
    ) -> Result<Self, DeviceDataError> {
        let mut definitions = Vec::with_capacity(entries.len());
        let mut source_names = Vec::with_capacity(entries.len());
        let mut index: HashMap<(u16, u16), usize> = HashMap::new();

        for (source_name, raw_toml) in entries {
            let def: DeviceDefinition =
                toml::from_str(&raw_toml).map_err(|source| DeviceDataError::TomlParse {
                    source_name: source_name.clone(),
                    source,
                })?;

            validate_definition(&def, &source_name, svg_base_dir)?;

            let slot = definitions.len();
            definitions.push(def);
            source_names.push(source_name.clone());

            for &pid in &definitions[slot].product_ids {
                if let Some(&existing_slot) = index.get(&(definitions[slot].vendor_id, pid)) {
                    let first_source = source_names
                        .get(existing_slot)
                        .cloned()
                        .unwrap_or_else(|| source_name.clone());
                    return Err(DeviceDataError::DuplicateProductId {
                        vendor_id: definitions[slot].vendor_id,
                        product_id: pid,
                        first_source,
                        second_source: source_name,
                    });
                }
                index.insert((definitions[slot].vendor_id, pid), slot);
            }
        }

        Ok(Self {
            definitions,
            sources: source_names,
            index,
        })
    }

    /// Returns all loaded [`DeviceDefinition`] entries.
    #[must_use]
    pub fn definitions(&self) -> &[DeviceDefinition] {
        &self.definitions
    }

    /// Returns source filenames corresponding 1:1 with [`Self::definitions`].
    #[must_use]
    pub fn source_names(&self) -> &[String] {
        &self.sources
    }

    /// Looks up a known device definition matching `(vendor_id, product_id)`.
    #[must_use]
    pub fn lookup(&self, vendor_id: u16, product_id: u16) -> Option<&DeviceDefinition> {
        self.index
            .get(&(vendor_id, product_id))
            .map(|&idx| &self.definitions[idx])
    }

    /// Resolves `(vendor_id, product_id)`, falling back to [`ResolvedDevice::Generic`].
    #[must_use]
    pub fn resolve(&self, vendor_id: u16, product_id: u16) -> ResolvedDevice<'_> {
        match self.lookup(vendor_id, product_id) {
            Some(def) => ResolvedDevice::Known(def),
            None => ResolvedDevice::Generic,
        }
    }

    /// Builds a standalone generic fallback profile when a device is unknown.
    #[must_use]
    pub fn generic_profile(
        vendor_id: u16,
        product_id: u16,
        hid_name: Option<&str>,
    ) -> GenericDeviceProfile {
        GenericDeviceProfile {
            vendor_id,
            product_id,
            name: ResolvedDevice::Generic.display_name(hid_name),
            kind: DeviceKind::Other,
        }
    }
}

fn validate_definition(
    def: &DeviceDefinition,
    source_name: &str,
    svg_base_dir: Option<&Path>,
) -> Result<(), DeviceDataError> {
    if def.name.trim().is_empty() {
        return Err(DeviceDataError::Validation {
            source_name: source_name.to_owned(),
            reason: "`name` must not be empty".to_owned(),
        });
    }
    if def.vendor_id == 0 {
        return Err(DeviceDataError::Validation {
            source_name: source_name.to_owned(),
            reason: "`vendor_id` must be non-zero".to_owned(),
        });
    }
    if def.product_ids.is_empty() {
        return Err(DeviceDataError::Validation {
            source_name: source_name.to_owned(),
            reason: "`product_ids` must contain at least one product ID".to_owned(),
        });
    }

    let mut seen_pids = HashSet::new();
    for &pid in &def.product_ids {
        if !seen_pids.insert(pid) {
            return Err(DeviceDataError::DuplicateProductId {
                vendor_id: def.vendor_id,
                product_id: pid,
                first_source: source_name.to_owned(),
                second_source: source_name.to_owned(),
            });
        }
    }

    let mut seen_cids = HashSet::new();
    let mut seen_svg_ids = HashSet::new();
    for btn in &def.buttons {
        if btn.label.trim().is_empty() {
            return Err(DeviceDataError::Validation {
                source_name: source_name.to_owned(),
                reason: format!("button cid 0x{:04X} has an empty label", btn.cid),
            });
        }
        if !seen_cids.insert(btn.cid) {
            return Err(DeviceDataError::DuplicateButtonCid {
                source_name: source_name.to_owned(),
                cid: btn.cid,
            });
        }
        if let Some(svg_id) = &btn.svg_id {
            if svg_id.trim().is_empty() {
                return Err(DeviceDataError::Validation {
                    source_name: source_name.to_owned(),
                    reason: format!("button cid 0x{:04X} has an empty svg_id", btn.cid),
                });
            }
            if !seen_svg_ids.insert(svg_id.clone()) {
                return Err(DeviceDataError::DuplicateButtonSvgId {
                    source_name: source_name.to_owned(),
                    svg_id: svg_id.clone(),
                });
            }
        }
    }

    if let Some(rel_svg) = &def.svg {
        validate_relative_svg_path(rel_svg, source_name)?;
        if let Some(base_dir) = svg_base_dir {
            let svg_path = base_dir.join(rel_svg);
            if !svg_path.is_file() {
                return Err(DeviceDataError::MissingSvgFile {
                    source_name: source_name.to_owned(),
                    svg_path,
                });
            }
            let svg_text = fs::read_to_string(&svg_path).map_err(|source| DeviceDataError::Io {
                path: svg_path.clone(),
                source,
            })?;
            let valid_ids = extract_svg_ids(&svg_text);
            for btn in &def.buttons {
                let Some(ref svg_id) = btn.svg_id else {
                    return Err(DeviceDataError::MissingSvgElementId {
                        cid: btn.cid,
                        svg_id: String::new(),
                        svg_path,
                    });
                };
                if !valid_ids.contains(svg_id) {
                    return Err(DeviceDataError::MissingSvgElementId {
                        cid: btn.cid,
                        svg_id: svg_id.clone(),
                        svg_path,
                    });
                }
            }
        }
    }

    Ok(())
}

fn validate_relative_svg_path(rel: &str, source_name: &str) -> Result<(), DeviceDataError> {
    let trimmed = rel.trim();
    if trimmed.is_empty() {
        return Err(DeviceDataError::Validation {
            source_name: source_name.to_owned(),
            reason: "`svg` must not be empty when specified".to_owned(),
        });
    }
    let path = Path::new(trimmed);
    let valid_components = path.components().all(|c| matches!(c, Component::Normal(_)));
    if path.is_absolute() || !valid_components {
        return Err(DeviceDataError::Validation {
            source_name: source_name.to_owned(),
            reason: format!("`svg` path {rel:?} must be a relative path without `..`"),
        });
    }
    Ok(())
}

/// Scans XML `id="..."` / `id='...'` attribute values outside XML comments.
#[must_use]
pub fn extract_svg_ids(svg: &str) -> HashSet<String> {
    let bytes = svg.as_bytes();
    let mut ids = HashSet::new();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i..].starts_with(b"<!--") {
            if let Some(end) = svg[i + 4..].find("-->") {
                i = i + 4 + end + 3;
                continue;
            }
            break;
        }

        // Attribute names are preceded by ASCII whitespace inside an XML tag.
        if i > 0
            && bytes[i - 1].is_ascii_whitespace()
            && bytes[i..].starts_with(b"id")
            && bytes
                .get(i + 2)
                .is_some_and(|&b| b == b'=' || b.is_ascii_whitespace())
        {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes.get(j) == Some(&b'=') {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if let Some(&quote @ (b'"' | b'\'')) = bytes.get(j) {
                    j += 1;
                    let start = j;
                    while j < bytes.len() && bytes[j] != quote {
                        j += 1;
                    }
                    if j < bytes.len()
                        && let Some(val) = svg.get(start..j)
                        && !val.is_empty()
                    {
                        ids.insert(val.to_owned());
                    }
                    i = j.saturating_add(1);
                    continue;
                }
            }
        }
        i += 1;
    }

    ids
}

/// Convenience map from button CID to human label for quick lookup.
#[must_use]
pub fn button_label_map(def: &DeviceDefinition) -> BTreeMap<u16, &str> {
    def.buttons
        .iter()
        .map(|b| (b.cid, b.label.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "squeak-devicedata-test-{prefix}-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn repo_data_devices_all_valid_and_in_sync_with_builtin() {
        let repo_devices_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("data")
            .join("devices");

        let disk_db = DeviceDatabase::load_from_dir(&repo_devices_dir).unwrap();
        let builtin_db = DeviceDatabase::builtin().unwrap();

        assert!(!disk_db.definitions().is_empty());
        assert_eq!(disk_db.definitions(), builtin_db.definitions());

        // Verify G502 HERO resolves cleanly
        let g502 = builtin_db.lookup(0x046D, 0xC08B).unwrap();
        assert_eq!(g502.name, "G502 HERO");
        assert_eq!(g502.kind, DeviceKind::Mouse);
        assert_eq!(button_label_map(g502).get(&0x0050), Some(&"Left click"));

        // Unknown VID:PID resolves to generic mode
        let generic = builtin_db.resolve(0x046D, 0xBEEF);
        assert!(!generic.is_known());
        assert_eq!(
            generic.display_name(Some("Unknown Super Mouse")),
            "Unknown Super Mouse"
        );
        assert_eq!(generic.display_name(None), "Generic HID++ 2.0 Device");
    }

    #[test]
    fn rejects_duplicate_pid_across_files() {
        let tmp = TempDir::new("dup-pid");
        fs::write(
            tmp.path.join("a.toml"),
            r#"
name = "Mouse A"
vendor_id = 0x046d
product_ids = [0xc08b]
kind = "mouse"
"#,
        )
        .unwrap();
        fs::write(
            tmp.path.join("b.toml"),
            r#"
name = "Mouse B"
vendor_id = 0x046d
product_ids = [0xc08b]
kind = "mouse"
"#,
        )
        .unwrap();

        let err = DeviceDatabase::load_from_dir(&tmp.path).unwrap_err();
        assert!(matches!(
            err,
            DeviceDataError::DuplicateProductId {
                vendor_id: 0x046D,
                product_id: 0xC08B,
                ..
            }
        ));

        let tmp_single = TempDir::new("dup-pid-single");
        fs::write(
            tmp_single.path.join("dup.toml"),
            r#"
name = "SelfDup"
vendor_id = 0x046d
product_ids = [0xc08b, 0xc08b]
kind = "mouse"
"#,
        )
        .unwrap();
        assert!(matches!(
            DeviceDatabase::load_from_dir(&tmp_single.path),
            Err(DeviceDataError::DuplicateProductId {
                vendor_id: 0x046D,
                product_id: 0xC08B,
                ..
            })
        ));
    }

    #[test]
    fn validates_svg_file_and_button_svg_ids() {
        let tmp = TempDir::new("svg-valid");
        fs::write(
            tmp.path.join("mouse.svg"),
            r#"<svg><!-- id="btn-commented-out" --><rect id="btn-left"/><path id='btn-right' data-id="ignore"/></svg>"#,
        )
        .unwrap();
        fs::write(
            tmp.path.join("ok.toml"),
            r#"
name = "OK Mouse"
vendor_id = 0x046d
product_ids = [0x1111]
kind = "mouse"
svg = "mouse.svg"

[[buttons]]
cid = 0x0050
label = "Left"
svg_id = "btn-left"

[[buttons]]
cid = 0x0051
label = "Right"
svg_id = "btn-right"
"#,
        )
        .unwrap();

        assert!(DeviceDatabase::load_from_dir(&tmp.path).is_ok());

        // Now add a button pointing at an ID only inside an XML comment
        fs::write(
            tmp.path.join("bad.toml"),
            r#"
name = "Bad Mouse"
vendor_id = 0x046d
product_ids = [0x2222]
kind = "mouse"
svg = "mouse.svg"

[[buttons]]
cid = 0x0050
label = "Missing"
svg_id = "btn-commented-out"
"#,
        )
        .unwrap();

        let err = DeviceDatabase::load_from_dir(&tmp.path).unwrap_err();
        assert!(matches!(err, DeviceDataError::MissingSvgElementId { .. }));
    }

    #[test]
    fn rejects_svg_path_traversal_and_duplicate_cids() {
        let tmp = TempDir::new("bad-paths");
        fs::write(
            tmp.path.join("traversal.toml"),
            r#"
name = "Traversal"
vendor_id = 0x046d
product_ids = [0x3333]
kind = "mouse"
svg = "../etc/passwd"
"#,
        )
        .unwrap();
        assert!(matches!(
            DeviceDatabase::load_from_dir(&tmp.path),
            Err(DeviceDataError::Validation { .. })
        ));

        let tmp2 = TempDir::new("dup-cid");
        fs::write(
            tmp2.path.join("dup.toml"),
            r#"
name = "Dup CID"
vendor_id = 0x046d
product_ids = [0x4444]
kind = "mouse"

[[buttons]]
cid = 0x0050
label = "One"

[[buttons]]
cid = 0x0050
label = "Two"
"#,
        )
        .unwrap();
        assert!(matches!(
            DeviceDatabase::load_from_dir(&tmp2.path),
            Err(DeviceDataError::DuplicateButtonCid { cid: 0x0050, .. })
        ));
    }
}
