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
  - Payload: `[orig_feature_idx, orig_func_sw, error_code, ...]`
  - Mapped to a typed `Hidpp20Error` (`InvalidArgument`, `OutOfRange`, `HwError`, `Busy`, etc.); `Busy` triggers bounded backoff retry.

## Runtime Discovery
1. Query `IRoot` (feature index `0x00`, feature ID `0x0000`), function `1` (`ping`) -> returns protocol `(major, minor)`. HID++ 1.0 devices respond with an error and are classified as unsupported legacy devices.
2. Query `IRoot` function `0` (`getFeature(0x0001)`) to locate `IFeatureSet` (index, type, version).
3. Query `IFeatureSet` function `0` (`getCount`) and iterate function `1` (`getFeatureID(index)`) to build the runtime feature table.
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

## Reference Sources (read-only; no copied code)
- Logitech public HID++ 2.0 protocol specification drafts
- Solaar (`lib/logitech_receiver/` — protocol behavior reference only)
- libratbag (`src/driver-hidpp20.c` — protocol behavior reference only)
