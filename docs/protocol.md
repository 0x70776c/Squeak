# HID++ 2.0 Protocol Notes

> **Scope:** HID++ 2.0+ only. HID++ 1.0 legacy devices are explicitly out of scope and fail cleanly during `IRoot` ping.

## Framing
- **Report IDs & Sizes:**
  - `0x10`: Short report (7 bytes)
  - `0x11`: Long report (20 bytes)
  - `0x12`: Very long report (64 bytes)
- **Header Layout (bytes 0..4):**
  - `[0]` `report_id` (`0x10` | `0x11` | `0x12`)
  - `[1]` `device_index` (`0xFF` for direct USB/BT; `1..=6` for receiver slots)
  - `[2]` `feature_index` (discovered at runtime; `0x00` is always `IRoot`; `0xFF` indicates an HID++ 2.0 error response)
  - `[3]` `(function_id << 4) | (sw_id & 0x0F)`
- **Software ID (`sw_id`):**
  - `0x0` is reserved for unsolicited device notifications/events.
  - Requests use a non-zero `sw_id` (`1..=15`) to correlate replies and discard unrelated notifications while awaiting a response.
- **Errors (`feature_index == 0xFF`):**
  - Wire layout: `[report_id, device_index, 0xFF, orig_feature_idx, orig_func_sw, error_code, ...]`
  - Mapped to a typed `Hidpp20Error` (`InvalidArgument`, `OutOfRange`, `HardwareError`, `Busy`, etc.); `Busy` triggers bounded backoff retry.
- **HID++ 1.0 Legacy & Receiver Errors (`sub_id == 0x8F` on `0x10` short frames):**
  - Wire layout: `[0x10, device_index, 0x8F, orig_sub_id, orig_func_sw, err10_code, 0x00]`
  - Receiver slot errors (`1..=6`) with `UnknownDevice`/`ResourceError`/`ConnectFail` map to `ProtocolError::ReceiverDeviceOffline` and `Busy` triggers bounded backoff retry; `InvalidSubId`/other legacy errors map to `ProtocolError::LegacyDeviceNotSupported`.

## Runtime Discovery
1. Query `IRoot` (feature index `0x00`, feature ID `0x0000`), function `1` (`ping`) -> returns protocol `(major, minor)`. HID++ 1.0 devices respond with an error and are classified as unsupported legacy devices.
2. Query `IRoot` function `0` (`getFeature(0x0001)`) to locate `IFeatureSet` (index, type, version).
3. Query `IFeatureSet` function `0` (`getCount`) and iterate function `1` (`getFeatureID(index)` for `1..=min(count, 0xFE)`) to build the runtime feature table (`0xFF` is reserved as the error sentinel). When firmware implements `IFeatureSet` v0, feature versions default to `0` regardless of long-report tail bytes.
4. Cache the discovered feature table per connected device instance; invalidate on disconnect/reconnect or firmware version change.

## Target Feature IDs (verify versions against hardware via `squeak dump-device`)
| Feature | Feature ID | Milestone |
|---|---|---|
| `IRoot` | `0x0000` | M1 |
| `IFeatureSet` | `0x0001` | M1 |
| `FIRMWARE_INFO` | `0x0003` | M3 |
| `DEVICE_NAME` / `TYPE` | `0x0005` | M3 |
| `BATTERY_STATUS` | `0x1000` | M3 |
| `UNIFIED_BATTERY` | `0x1004` | M3 |
| `REPROG_CONTROLS_V4` | `0x1B04` | M5 |
| `SMART_SHIFT` | `0x2110` | M5 |
| `HIRES_WHEEL` | `0x2121` | M5 |
| `ADJUSTABLE_DPI` | `0x2201` | M3 |
| `REPORT_RATE` | `0x8060` | M3 |
| `EXTENDED_REPORT_RATE` | `0x8061` | M3 |
| `COLOR_LED_EFFECTS` | `0x8070` | M5 |
| `RGB_EFFECTS` | `0x8071` | M5 |
| `ONBOARD_PROFILES` | `0x8100` | M6 |

## M3 Feature Wire Notes
- **`0x0003` (`FIRMWARE_INFO`, v0..=v4):**
  - `fn0` `getDeviceInfo`: returns `entityCount`, optional `unitId` (v1+, bytes `[1..5]` — **always discarded** for privacy), `transport` flags (byte `5`, bitmask `BTLE | eQuad | BT | USB`), `modelId` (`[6..12]`), and `extendedModelId` (byte `12`).
  - `fn1` `getFwInfo(entityIdx)`: returns `type & 0x0F` (MainApplication, Bootloader, Hardware, Touchpad, SoftDevice, RfCompanion, FactoryApplication, HexFile), 3-byte ASCII prefix (`[1..4]`), BCD `fw_number` (`[4]`), BCD `revision` (`[5]`), BE `u16` `build` (`[6..8]`), `active` (`[8] & 0x01`, MainApplication/SoftDevice), and `trPid` (`[9..11]` BE `u16`).
- **`0x0005` (`DEVICE_NAME`, v0):**
  - `fn0` `getDeviceNameCount`: returns byte length (`params[0]`).
  - `fn1` `getDeviceName(charIndex)`: returns UTF-8 chunk directly in `params[..]` (no leading offset byte); assemble up to declared count.
  - `fn2` `getDeviceType`: returns enum byte (`0..=9`: Keyboard, RemoteControl, Numpad, Mouse, Trackpad, Trackball, Presenter, Receiver, Headset, Webcam).
- **`0x1000` (`BATTERY_STATUS`, v0..=v1) & `0x1004` (`UNIFIED_BATTERY`, v0..=v5):**
  - `0x1000` `fn0` (`getBatteryLevelStatus`): `[level%, next_level%, charging_status]`; `fn1` (`getBatteryCapability`): flags & nominal voltage / level count. Fallback heuristic when `0x1004` is absent.
  - `0x1004` `fn0` (`getCapabilities`): `[supported_levels_mask, flags]`; `fn1` (`getStatus`): `[state_of_charge (0..=100), battery_level_bucket, charging_status, external_power]`. Preferred over `0x1000` when both exist.
- **`0x2201` (`ADJUSTABLE_DPI`, v0..=v2):**
  - `fn0` `getSensorCount`: `[count]`
  - `fn1` `getSensorDpiList(sensorIdx)`: `[sensor_idx, u16_be... , 0x0000]`. Stepped ranges encode as `[min, 0xE000 | step, max]`; max protocol DPI is `51200` (`> 0xC800` / `< 0xE000`).
  - `fn2` `getSensorDpi(sensorIdx)`: `[sensor_idx, dpi_hi, dpi_lo, default_dpi_hi, default_dpi_lo]` (`default_dpi == 0` in v0).
  - `fn3` `setSensorDpi(sensorIdx, dpi)`: `[sensor_idx, dpi_hi, dpi_lo]` (echoes on v1+, no echo on v0). Always followed by mandatory `getSensorDpi` readback verification.
- **`0x8060` (`REPORT_RATE`, v0) & `0x8061` (`EXTENDED_REPORT_RATE`, v0):**
  - `0x8060`: `fn0` `getReportRateList` -> 8-bit bitmask (`bit i` = `(i + 1)` ms); `fn1` `getReportRate` -> `ms` (`1..=8`); `fn2` `setReportRate(ms)`.
  - `0x8061`: `fn1` `getReportRateList` -> BE `u16` bitmask (`bit 0` = 8 ms / 125 Hz .. `bit 6` = 125 us / 8000 Hz); `fn2` `getReportRate` -> code (`0..=6`); `fn3` `setReportRate(code)`.

## Reference Sources (read-only; no copied code)
- Logitech public HID++ 2.0 protocol specification drafts (`x0003_deviceinfo.html`, `x2201_adjustabledpi.html` via `Logitech/lekensteyn` HID++ 2.0 specification repository)
- Solaar (`lib/logitech_receiver/hidpp20.py`, `descriptors.py`, `settings_templates.py` — protocol behavior reference only)
- libratbag (`src/driver-hidpp20.c` — protocol behavior reference only)
