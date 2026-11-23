# Multimeter WebSocket API

Protocol version: `1`

The executable exposes one local WebSocket session for both control commands and
periodic hardware updates. The server binds to `127.0.0.1` on an ephemeral port
and uses a random token generated for each launch.

The executable probes `sysinfo` and NVML at startup, opens the configured
frontend with a launch token, and serves fresh snapshots on demand. There is no
background update service; seeking drives repeated snapshot requests from the
session loop.

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
    "providers": {
      "sysinfo": {
        "available": true
      },
      "nvml": {
        "available": false,
        "reason": "no_supported_gpu"
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
section. Available sections are `platform`, `cpu`, `memory`, `swap`, `disks`,
`networks`, `gpus`, and `components`.

```json
{
  "type": "command",
  "id": "query-1",
  "command": "info.query",
  "params": {
    "sections": ["cpu", "memory", "gpus"]
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
        "frequency_mhz": 3600
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
values are `null`. A successful query may include warnings when an optional
provider or section is unavailable.

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
    "sections": ["cpu", "memory", "gpus", "components"]
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
      "frequency_mhz": 3600
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

### `app.shutdown`

Stops collection and terminates the API process.

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

Malformed JSON and unsupported message types do not close an authenticated
session. Authentication failures, oversized authentication messages, origin
rejections, and connection-slot conflicts close the WebSocket.

## State And Disconnect Behavior

Valid states are `idle`, `seeking`, `paused`, and `closing`.

If the browser disconnects while seeking, the server records the session as
`paused`. Reconnecting with the same launch token reports the paused state, and
the page may call `seeking.start` again. A disconnected session does not exit
the process automatically; `app.shutdown`, process interruption, or the
operating system ends it.
