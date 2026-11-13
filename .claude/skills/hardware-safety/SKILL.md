---
name: hardware-safety
description: Mandatory safety rules before any HID++ device write, readback verification, or onboard memory operation (M3-M6+).
---

# Hardware Safety Rules

## 1. Ask-First Gate on Real Hardware
- **Never** execute a write command against physical hardware without explicit user confirmation before the first real write for a milestone/feature.
- All unit tests and CI checks must use `MockTransport`; hardware verification is strictly opt-in and local.

## 2. Dry-Run & Read-Only Guards
- Every mutating CLI subcommand must support `--dry-run` (resolving the target feature, validating input bounds against live/read capability data, and printing planned frames/values without transmitting write requests).
- Unknown feature revisions (`reported_version > MAX_SUPPORTED_VERSION`) must degrade to read-only and refuse writes with a typed error (`ProtocolError::UnsupportedFeatureVersion` or equivalent).

## 3. Pre-Write Snapshot & Bounds Validation
- Before writing DPI (`0x2201`) or report rate (`0x8060` / `0x8061`), query the device's supported capability range/list/bitmask and validate the requested value against it.
- Before any interactive test write on real hardware, read and record the current active state so the previous configuration can be restored immediately.

## 4. Mandatory Readback Verification
- Every device write helper that mutates runtime settings must perform an immediate readback query after the write and verify that the device state matches the requested value (within documented sensor rounding/step constraints), returning a typed mismatch error if it does not.

## 5. Onboard Memory Lock (M6+)
- Non-volatile onboard profile flash writes (`ONBOARD_PROFILES` `0x8100`) are forbidden in M3–M5.
- Starting in M6, onboard memory writes require a complete sector/directory backup, CRC verification before and after write, a tested restore path, and explicit user confirmation before touching real hardware.
