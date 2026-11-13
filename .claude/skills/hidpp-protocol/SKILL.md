---
name: hidpp-protocol
description: Use when implementing or debugging HID++ 2.0 framing, feature discovery, or any feature module (DPI, LEDs, buttons, profiles, battery) in the hidpp crate.
---

# HID++ 2.0 reference

## Framing
- Report IDs: 0x10 short (7 B), 0x11 long (20 B), 0x12 very long (64 B)
- Layout: `[report_id, device_index, feature_index, (function<<4 | sw_id), params...]`
- `device_index`: 0xFF = direct (USB/BT), 1..6 = receiver slot
- `sw_id`: 1..15, pick a fixed non-zero id for requests; use it to filter responses
  (notifications have `sw_id` 0)
- Errors: `feature_index` 0xFF, params: `[orig_feature_idx, orig_func_sw, err_code]`
  -> map to a typed `Hidpp20Error` (`InvalidArgument`, `OutOfRange`, `HwError`, `Busy`, ...)
- Timeouts + retry on `Busy` with backoff; ignore unrelated notifications while awaiting reply

## Discovery
- `IRoot` is always feature index 0x00 (ID 0x0000): fn0 `getFeature(featureId)` -> index,type,version;
  fn1 `ping` -> protocol version (`major.minor`). Err on 1.0 -> "legacy device".
- `IFeatureSet` 0x0001: fn0 `getCount`, fn1 `getFeatureID(index)`
- Cache the table per device; invalidate on reconnect/firmware change.

## Feature IDs (verify against hardware via `dump-device`, versions differ!)
| Feature | ID |
|---|---|
| DEVICE_NAME / TYPE | 0x0005 |
| FIRMWARE_INFO | 0x0003 |
| BATTERY_STATUS / UNIFIED_BATTERY | 0x1000 / 0x1004 |
| REPROG_CONTROLS_V4 | 0x1B04 |
| ADJUSTABLE_DPI | 0x2201 |
| REPORT_RATE / EXTENDED_REPORT_RATE | 0x8060 / 0x8061 |
| COLOR_LED_EFFECTS | 0x8070 |
| RGB_EFFECTS | 0x8071 |
| ONBOARD_PROFILES | 0x8100 |
| HIRES_WHEEL / SMART_SHIFT | 0x2121 / 0x2110 |

## Implementation pattern
- Each feature = module with `trait` + versioned impl, parsing from `&[u8]` into typed structs
  (newtypes for DPI, percent, RGB). Pure functions, no I/O inside parsers.
- Golden tests: byte arrays -> structs and back. Add property tests (`proptest`) for roundtrips.
- Unknown feature version -> degrade gracefully (read-only / hide), log at `warn`.
- References (read, do not copy): Solaar `lib/logitech_receiver/`, libratbag `src/driver-hidpp20.c`,
  Logitech public HID++ spec drafts. Record sources in `docs/protocol.md`.
