# Squeak D-Bus API Specification (`dev.squeak.Daemon`)

- **Well-known Bus Name:** `dev.squeak.Daemon`
- **Object Path:** `/dev/squeak/Daemon`
- **Interface Name:** `dev.squeak.Daemon`
- **Bus Type:** User Session D-Bus (`DBUS_SESSION_BUS_ADDRESS`)
- **API Version:** `1`

The `squeakd` background daemon acts as the **exclusive single owner** of `/dev/hidraw*` devices on Linux. Unprivileged client tools (`squeak-cli`, `squeak-gui`) communicate with hardware solely through this interface to prevent multi-process device contention.

---

## 1. Properties

| Property | Type | Access | Description |
|---|---|---|---|
| `ApiVersion` | `u` (`u32`) | Read | Current D-Bus protocol version (`1`). |
| `DaemonVersion` | `s` (`String`) | Read | `CARGO_PKG_VERSION` of the running daemon. |

---

## 2. Methods

### `ListDevices() -> (s)`
Returns a JSON-serialized array of `DaemonDeviceSummary` objects describing all currently tracked HID++ endpoints.

- **Returns:** JSON string (e.g. `[{"device_key": "hidraw6-046d-c08b-ff", ...}]`).
- **Privacy:** `HID_UNIQ` and per-unit serial numbers are strictly excluded.

### `GetDeviceStatus(s device_key) -> (s)`
Reads live M3 status for the device matching `device_key` (or selector like `046d:c08b` / `hidraw6`).

- **Parameters:**
  - `device_key`: Target identifier, sysfs node name, or `vid:pid`.
- **Returns:** JSON-serialized `DeviceStatusReport`.
- **Errors:** `org.freedesktop.DBus.Error.InvalidArgs` if device is unknown, asleep, or selector ambiguous.

### `DumpDevice(s device_key) -> (s)`
Returns the complete sanitized runtime feature table (`DeviceDump`) without serial numbers.

- **Parameters:**
  - `device_key`: Target identifier, sysfs node name, or `vid:pid`.
- **Returns:** JSON-serialized `DeviceDump`.

### `SetDpi(s device_key, y sensor_index, q dpi, b dry_run) -> (s)`
Validates, previews (`dry_run = true`), or writes and readback-verifies (`dry_run = false`) the active sensor DPI.

- **Parameters:**
  - `device_key`: Target device selector.
  - `sensor_index`: Zero-based sensor index (`0` for single-sensor mice).
  - `dpi`: Requested DPI value (validated against device capabilities).
  - `dry_run`: `true` to preview the planned wire frame without sending writes; `false` to commit.
- **Returns:** JSON-serialized `DpiWriteReport`.
- **Emits:** `DeviceSettingsApplied` upon verified write; `DeviceError` on failure.

### `SetReportRate(s device_key, q hz, b dry_run) -> (s)`
Validates, previews (`dry_run = true`), or writes and readback-verifies (`dry_run = false`) the USB polling rate.

- **Parameters:**
  - `device_key`: Target device selector.
  - `hz`: Requested rate in Hz (`125`, `250`, `500`, `1000`, `2000`, `4000`, `8000`).
  - `dry_run`: `true` to preview; `false` to commit.
- **Returns:** JSON-serialized `ReportRateWriteReport`.
- **Emits:** `DeviceSettingsApplied` upon verified write; `DeviceError` on failure.

### `RefreshDevices() -> (u)`
Forces a rescan of sysfs `/dev/hidraw*` endpoints and prunes disconnected devices.

- **Returns:** Total number of currently tracked devices.
- **Emits:** `DevicesChanged` if devices were added or removed.

### `HandleWake() -> (u)`
Invalidates all cached feature tables and sessions, re-probes connected endpoints, and re-applies stored settings (called automatically on `systemd-logind` resume).

- **Returns:** Number of successfully refreshed and accessible devices.

---

## 3. Signals

### `DevicesChanged(as added, as removed)`
Emitted when `hidraw` devices are connected or disconnected via netlink uevents or manual refresh.

- `added`: Array of newly registered `device_key` strings.
- `removed`: Array of unplugged `device_key` strings.

### `DeviceSettingsApplied(s device_key, s setting)`
Emitted after a setting is successfully committed and readback-verified on hardware (e.g. `setting = "dpi:0=1600"` or `"report_rate=1000Hz"`).

### `DeviceError(s device_key, s message)`
Emitted when an asynchronous hotplug re-apply or device communication operation encounters an error.

---

## 4. Systemd & D-Bus Activation

- **D-Bus Service File:** `data/dbus/dev.squeak.Daemon.service`
  ```ini
  [D-BUS Service]
  Name=dev.squeak.Daemon
  Exec=/usr/bin/squeakd
  SystemdService=squeakd.service
  ```
- **Systemd User Unit:** `data/systemd/squeakd.service`
  ```ini
  [Unit]
  Description=Squeak HID++ Hardware Daemon
  Documentation=https://github.com/0x70776c/Squeak

  [Service]
  ExecStart=/usr/bin/squeakd
  Restart=on-failure
  RestartSec=2s

  [Install]
  WantedBy=default.target
  ```
