# Device Support

Squeak discovers HID++ 2.0 features dynamically from connected hardware using `IRoot` (`0x0000`) and `IFeatureSet` (`0x0001`). Even devices without a dedicated definition file work via Squeak's generic HID++ 2.0 fallback.

## Verified & First-Class Devices

| Device | VID:PID | Transport | Supported Capabilities | Definition |
| --- | --- | --- | --- | --- |
| G502 HERO | `046d:c08b` | Wired USB (`hidraw`) | DPI (`0x2201`), Report Rate (`0x8060`), RGB LEDs (`0x8070`), Onboard Profiles (`0x8100`), Buttons (`0x8110`/`0x1B04`), Interactive SVG | [g502-hero.toml](file:///home/paweldev/Downloads/Antigravity-x64/Projectts/Squeak/data/devices/g502-hero.toml) |

## Generic HID++ 2.0 Fallback

Any HID++ 2.0 mouse or receiver endpoint exposing standard HID++ usage pages (`0xFF00`, `0xFF43`) is automatically probed and supported for all discovered standard features (DPI, polling rate, battery, RGB effects, onboard profiles, and button controls) using `generic-mouse.svg`.

## Privacy & Serial Number Policy

Squeak never reads, logs, stores, or exports `HID_UNIQ` or device hardware serial numbers (`0x0003` unit IDs are redacted).
