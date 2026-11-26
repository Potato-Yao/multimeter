# Multimeter WebSocket API

Protocol version: `1`

The executable exposes one local WebSocket session for both control commands and
periodic hardware updates. The server binds to `127.0.0.1` on an ephemeral port
and uses a random token generated for each launch.

The executable probes `sysinfo`, CPU package-power telemetry, NVML, and system
batteries at startup, opens the configured frontend with a launch token, and
serves fresh snapshots on demand. There is no background update service; seeking
drives repeated snapshot requests from the session loop.

## Elevated startup

Multimeter only runs with administrator permissions. At startup it checks the
effective user before probing providers, opening a port, or launching the
frontend.

- On Linux and macOS, a non-root instance asks `sudo` to authenticate and then
  launches the same executable with `sudo`. The original instance exits only
  after the elevated launch succeeds.
- On Windows, a non-elevated process requests elevation through the Windows
  `runas` operation and therefore displays the normal administrator/UAC prompt.
  The original instance exits only after Windows accepts the request.

Cancelled authentication, denied elevation, unavailable elevation tools, and
other launch failures stop multimeter with an error. The process never falls
back to collecting and serving information with normal user permissions.

## Build-time configuration

The frontend base URL is selected when the executable is built. The project
default is configured in `.cargo/config.toml`:

```toml
[env]
MULTIMETER_BASE_URL = "https://clinic.bitnp.net/test"
```

Override it for a build with a process environment variable:

```sh
MULTIMETER_BASE_URL=https://clinic.bitnp.net/test cargo build --release
```

`build.rs` validates that the value is non-empty, contains no whitespace, and
uses `http://` or `https://`. The value is embedded at compile time and exposed
as `multimeter::config::FRONTEND_BASE_URL`. It is not read from the end-user's
environment at runtime.

### Windows shader compiler variants

The default build embeds the DirectX Shader Compiler (DXC) in the Windows
executable. This self-contained build does not require a separate shader
compiler at runtime:

```powershell
cargo build --release
```

Build the smaller variant without default features when the target machine can
provide `dxcompiler.dll` or the system DirectX shader compiler
(`d3dcompiler_47.dll`):

```powershell
cargo build --release --no-default-features
```

The two release assets are:

- `multimeter-<tag>-x86_64-pc-windows-msvc.exe`: self-contained with static DXC.
- `multimeter-<tag>-x86_64-pc-windows-msvc-no-static-dxc.exe`: smaller build
  without static DXC.

If neither runtime compiler is available, measurement and CPU testing still
work, but GPU compute testing cannot initialize. Linux and Windows ARM64 builds
do not include static DXC.

## Local websocat testing

Build the executable as the normal user, then use the platform-specific local
test launcher. Do not run Cargo itself with administrator privileges.

On Linux:

```sh
./scripts/test-local.sh
```

The script requests `sudo` elevation, launches the executable with
`--no-browser`, extracts the generated WebSocket URL and authentication token
from the launch URL, sends the authentication frame, and opens an interactive
`websocat` session. Pass `--executable PATH` when the binary is not in a normal
Cargo output directory.

On Windows, run:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\test-local.ps1
```

The script requests UAC elevation through PowerShell and otherwise provides the
same local `websocat` workflow. Pass `-Executable PATH` when needed.

Both scripts stop multimeter when `websocat` exits. Input is one JSON message
per line, for example:

```json
{"type":"command","id":"query-1","command":"info.query","params":{"sections":["cpu","memory"]}}
```

The `--no-browser` executable option suppresses the automatic frontend launch;
normal startup still opens the configured frontend URL.

## Bootstrap

The page receives a URL-safe, base64-encoded JSON payload in the URL fragment:

```text
<MULTIMETER_BASE_URL>#multimeter=<base64url({
  "v": 1,
  "ws": "ws://127.0.0.1:49123/v1/ws",
  "token": "<random token>"
})>
```

The fragment is not sent to the frontend server as part of the HTTP request.
The page opens `ws` and sends the token as its first WebSocket message.

```json
{
  "type": "authenticate",
  "token": "<random token>"
}
```

The server closes the connection if authentication does not complete within
the configured timeout or the token does not match. Only one client may hold the
active session. The token remains valid for the process lifetime, so the page
may reconnect after an ordinary disconnect.

When `ApiOptions.allowed_origin` is configured, the server also requires the
WebSocket `Origin` header to match it.

## Session Ready

After successful authentication:

```json
{
  "type": "event",
  "event": "session.ready",
  "data": {
    "protocol_version": 1,
    "state": "idle",
    "cpu_test": {
      "state": "idle"
    },
    "providers": {
      "sysinfo": {
        "available": true
      },
      "nvml": {
        "available": false,
        "reason": "no_supported_gpu"
      },
      "cpu_power": {
        "available": true
      },
      "battery": {
        "available": true
      }
    },
    "seeking": {
      "default_interval_ms": 1000,
      "minimum_interval_ms": 250,
      "maximum_interval_ms": 60000
    }
  }
}
```

Provider unavailability is reported as a capability and does not prevent the
session from starting.

## Commands

Each command must contain a non-empty request identifier. Responses repeat that
identifier.

Ready-to-paste interactive examples are available in
[QUERY_EXAMPLES.md](QUERY_EXAMPLES.md).

```json
{
  "type": "command",
  "id": "cmd-1",
  "command": "info.query",
  "params": {}
}
```

Successful response:

```json
{
  "type": "response",
  "id": "cmd-1",
  "ok": true,
  "result": {}
}
```

Failed response:

```json
{
  "type": "response",
  "id": "cmd-1",
  "ok": false,
  "error": {
    "code": "invalid_request",
    "message": "interval_ms must be between 250 and 60000"
  }
}
```

### `info.query`

Collects one fresh snapshot. Omitting `sections` requests every available
section. Available sections are `platform`, `cpu`, `self`, `memory`, `swap`,
`disks`, `networks`, `gpus`, `components`, and `batteries`.

```json
{
  "type": "command",
  "id": "query-1",
  "command": "info.query",
  "params": {
    "sections": ["cpu", "self", "memory", "gpus"]
  }
}
```

The result has the form:

```json
{
  "type": "response",
  "id": "query-1",
  "ok": true,
  "result": {
    "snapshot": {
      "sequence": 1,
      "captured_at": "2026-11-23T12:34:56.789Z",
      "cpu": {
        "brand": "Example CPU",
        "physical_cores": 8,
        "logical_cores": 16,
        "usage_percent": 31.5,
        "frequency_mhz": 3600,
        "power_watts": 38.4
      },
      "self": {
        "pid": 12345,
        "cpu_usage_percent": 12.5,
        "memory_bytes": 67108864
      },
      "memory": {
        "total_bytes": 34359738368,
        "used_bytes": 12884901888,
        "available_bytes": 21474836480
      },
      "warnings": []
    }
  }
}
```

Values with explicit unit suffixes are stable API fields. Missing optional
values are omitted. A successful query may include warnings when an optional
provider or section is unavailable.

### Multimeter process usage

The `self` section reports resource usage for the running Multimeter process,
not the machine-wide totals.

```json
{
  "type": "command",
  "id": "query-self",
  "command": "info.query",
  "params": {
    "sections": ["self"]
  }
}
```

```json
{
  "type": "response",
  "id": "query-self",
  "ok": true,
  "result": {
    "snapshot": {
      "sequence": 3,
      "captured_at": "2026-11-23T12:34:58.789Z",
      "self": {
        "pid": 12345,
        "cpu_usage_percent": 12.5,
        "memory_bytes": 67108864
      },
      "warnings": []
    }
  }
}
```

`cpu_usage_percent` is normalized across all logical CPUs, so `100` means the
whole machine is saturated by Multimeter. `memory_bytes` is the process's
resident physical RAM. CPU usage needs two samples, so the first reading after
Multimeter starts may be `0.0`; periodic `seeking.start` snapshots update it.

### Power telemetry

`cpu.power_watts` reports total processor package power across sockets. On
Linux it is derived from the package-level RAPL `energy_uj` counters. On
Windows 11 it comes from the `Energy Meter` RAPL package counters and is cached
to match their approximately one-second update interval.

CPU power from an energy counter is an average over the interval since the
previous successful sample. The first CPU snapshot after collection starts or
resumes omits `power_watts` while a baseline is established; the next snapshot
normally includes it. Unsupported hardware produces no value and a
`cpu_power_unavailable` warning rather than failing the snapshot.

`gpus[].power_watts` is the GPU device power reported by NVML for NVIDIA GPUs.
Its averaging window and measurement scope are determined by the GPU and
driver. GPU power remains optional even when the `nvml` provider is available.
GPU telemetry for non-NVIDIA devices is not currently supported.

### Battery telemetry

The `batteries` section uses the cross-platform API from
[`starship/rust-battery`](https://github.com/starship/rust-battery). Each array
entry describes one physical battery:

```json
{
  "type": "command",
  "id": "query-batteries",
  "command": "info.query",
  "params": {
    "sections": ["batteries"]
  }
}
```

```json
{
  "type": "response",
  "id": "query-batteries",
  "ok": true,
  "result": {
    "snapshot": {
      "sequence": 4,
      "captured_at": "2026-11-27T08:30:00.000Z",
      "batteries": [
        {
          "index": 0,
          "vendor": "Example Vendor",
          "model": "Example Battery",
          "technology": "lithium-ion",
          "state": "discharging",
          "state_of_charge_percent": 73.4,
          "state_of_health_percent": 91.2,
          "energy_wh": 41.2,
          "energy_full_wh": 56.3,
          "energy_full_design_wh": 61.8,
          "energy_rate_watts": 12.7,
          "voltage_volts": 12.1,
          "temperature_celsius": 31.4,
          "cycle_count": 142,
          "time_to_empty_seconds": 11640
        }
      ],
      "warnings": []
    }
  }
}
```

`technology` is one of the values exposed by the battery provider, such as
`lithium-ion` or `unknown`. `state` is one of `unknown`, `charging`,
`discharging`, `empty`, `full`, or `paused`.

Charge and health percentages are clamped to `0..=100`. Energy is reported in
watt-hours, rate in watts, voltage in volts, and remaining time in seconds.
`energy_rate_watts` is a non-negative magnitude; use `state` to distinguish
charging from discharging. The time fields are present only when the current
state supports an estimate. Optional metadata and measurements are omitted when
the firmware or operating system does not provide them. Serial numbers are not
exposed.

Battery capability is determined at startup. Systems without a battery report
`providers.battery.available: false` with reason `no_battery_detected`; a queried
`batteries` section then contains an empty array and a `no_battery_detected`
warning. Provider failures produce `battery_unavailable` or
`battery_read_failed` warnings without failing the entire snapshot. A failure
reading one battery does not discard readings from other batteries.

### `seeking.start`

Starts periodic snapshots, or updates the active configuration without dropping
the session.

```json
{
  "type": "command",
  "id": "start-1",
  "command": "seeking.start",
  "params": {
    "interval_ms": 1000,
    "sections": ["cpu", "self", "memory", "gpus", "components", "batteries"]
  }
}
```

`interval_ms` defaults to `1000` and must be between `250` and `60000`. Omitting
`sections` preserves the current selection. Starting while already seeking
causes an immediate snapshot and restarts the interval from that point.

The command produces a response, a `seeking.state` event, and an immediate
`info.snapshot`. Thereafter snapshots arrive once per interval:

```json
{
  "type": "event",
  "event": "info.snapshot",
  "data": {
    "sequence": 2,
    "captured_at": "2026-11-23T12:34:57.789Z",
    "cpu": {
      "logical_cores": 16,
      "usage_percent": 25.4,
      "frequency_mhz": 3600,
      "power_watts": 38.4
    },
    "warnings": []
  }
}
```

Updates are not queued without bound. A slow client causes the next sample to be
coalesced rather than accumulating an ever-growing backlog.

### `seeking.pause`

Stops periodic snapshots while keeping the session, token, and current seeking
configuration alive.

```json
{
  "type": "command",
  "id": "pause-1",
  "command": "seeking.pause",
  "params": {}
}
```

After the response and `seeking.state: paused` event, no further
`info.snapshot` events are sent. `info.query` remains available, and
`seeking.start` resumes collection.

`seeking.start` while already seeking and `seeking.pause` while paused are
idempotent successes.

### `cpu_test.start`

Starts an infinite CPU burn with one worker per logical CPU. The command returns
immediately after the workers start; call `cpu_test.stop` when the test should
end.

```json
{
  "type": "command",
  "id": "cpu-start-1",
  "command": "cpu_test.start",
  "params": {}
}
```

Successful response:

```json
{
  "type": "response",
  "id": "cpu-start-1",
  "ok": true,
  "result": {
    "state": "running",
    "worker_count": 16
  }
}
```

The response is followed by:

```json
{
  "type": "event",
  "event": "cpu_test.state",
  "data": {
    "state": "running"
  }
}
```

Starting while a test is already running is an idempotent success. It does not
start additional workers or emit another state event.

### `cpu_test.stop`

Stops the active CPU burn and returns the operations completed so far.

```json
{
  "type": "command",
  "id": "cpu-stop-1",
  "command": "cpu_test.stop",
  "params": {}
}
```

Successful response:

```json
{
  "type": "response",
  "id": "cpu-stop-1",
  "ok": true,
  "result": {
    "state": "idle",
    "result": {
      "worker_count": 16,
      "batch_size": 1024,
      "elapsed_ms": 5021,
      "multiply_count": 17809121792,
      "multiplies_per_second": 3546896813.5
    }
  }
}
```

The response is followed by:

```json
{
  "type": "event",
  "event": "cpu_test.state",
  "data": {
    "state": "idle"
  }
}
```

Stopping while no test is running is an idempotent success with no `result`
field. CPU test state is independent of `seeking` state. CPU workers count a
multiplication only after completing a full batch.

### `app.shutdown`

Stops collection, stops any active CPU test, and terminates the API process.

```json
{
  "type": "command",
  "id": "shutdown-1",
  "command": "app.shutdown",
  "params": {}
}
```

The server sends, in order:

1. A successful response with state `closing`.
2. A `seeking.state` event with state `closing`.
3. A `session.closed` event with reason `shutdown`.
4. A WebSocket close frame.

The process then exits successfully.

## Other Events

An unavailable source during periodic collection emits:

```json
{
  "type": "event",
  "event": "info.error",
  "data": {
    "code": "provider_error",
    "message": "provider failed to collect a snapshot"
  }
}
```

Malformed messages that cannot be correlated to a request emit
`protocol.error`:

```json
{
  "type": "event",
  "event": "protocol.error",
  "data": {
    "code": "invalid_json",
    "message": "message must be valid JSON"
  }
}
```

## Errors

- `invalid_request`
- `unknown_command`
- `invalid_json`
- `request_too_large`
- `cpu_test_start_failed`
- `cpu_test_stop_failed`

Malformed JSON and unsupported message types do not close an authenticated
session. Authentication failures, oversized authentication messages, origin
rejections, and connection-slot conflicts close the WebSocket.

## State And Disconnect Behavior

Valid states are `idle`, `seeking`, `paused`, and `closing`.

If the browser disconnects while seeking, the server records the session as
`paused`. Reconnecting with the same launch token reports the paused state, and
the page may call `seeking.start` again.

CPU test state is reported separately as `idle` or `running`. An active CPU
test is cancelled automatically when its WebSocket session disconnects or the
process shuts down. Reconnecting starts with CPU test state `idle`.

A disconnected session does not exit the process automatically;
`app.shutdown`, process interruption, or the operating system ends it.
