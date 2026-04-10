# Volva Hook Adapter With Cortina

Use this slice when `volva` should forward its normalized hook events into the
real `cortina` adapter boundary:

```text
volva run ... -> cortina adapter volva hook-event
```

## Production Config

`volva` loads `volva.json` from the directory where you run the CLI. A minimal
`cortina` hook-adapter configuration looks like this:

```json
{
  "model": "claude-sonnet-4-6",
  "api_base_url": "https://api.anthropic.com",
  "experimental_bridge": false,
  "backend": {
    "kind": "official-cli",
    "command": "claude"
  },
  "hook_adapter": {
    "enabled": true,
    "command": "cortina",
    "args": ["adapter", "volva", "hook-event"]
  }
}
```

If `cortina` is not on `PATH`, keep the same argv and replace `"command":
"cortina"` with the absolute binary path.

For slower wrapper commands such as `cargo run --manifest-path .../cortina/Cargo.toml --`,
set `"timeout_ms"` higher than the default `30000`.

## Fast Config Check

From the directory that contains `volva.json`:

```bash
cargo run -p volva-cli -- backend status
cargo run -p volva-cli -- doctor
```

Expected output includes the configured adapter command line:

```text
hook_adapter: configured-external:cortina adapter volva hook-event
```

`backend status` is the shortest check. `doctor` prints the same adapter status
alongside the rest of the runtime surface, including:

```text
hook_adapter_timeout_ms: 30000
hook_adapter_command_resolved: true
```

When the configured adapter argv is the supported `cortina adapter volva
hook-event` surface, `backend doctor` also probes `cortina status --json --cwd`
and `cortina doctor --json --cwd` for the current working directory and prints
observed downstream health such as:

```text
hook_delivery_probe: cortina-status-doctor
hook_delivery_seen_for_cwd: true
hook_delivery_event_count: 4
hook_delivery_events_valid_json: true
```

Those lines prove what `cortina` sees for the current cwd. They do not turn
`volva` into a second lifecycle store.

## Manual Smoke Test

This smoke test proves three things:

1. `volva run` emits normalized hook events
2. `cortina adapter volva hook-event` accepts them
3. `cortina` records them in scoped temp-state JSON and surfaces that state
   through `status` and `doctor`

The fast path below avoids a real Claude install by using `/bin/echo` as the
official backend command while keeping the hook-adapter wiring unchanged.

### 1. Build `cortina`

```bash
cd /Users/williamnewton/projects/basidiocarp/cortina
cargo build
export PATH="/Users/williamnewton/projects/basidiocarp/cortina/target/debug:$PATH"
```

### 2. Configure `volva` for the smoke run

Use this `volva.json` in the working directory where you will run `volva`:

```json
{
  "model": "claude-sonnet-4-6",
  "api_base_url": "https://api.anthropic.com",
  "experimental_bridge": false,
  "backend": {
    "kind": "official-cli",
    "command": "/bin/echo"
  },
  "hook_adapter": {
    "enabled": true,
    "command": "cortina",
    "args": ["adapter", "volva", "hook-event"],
    "timeout_ms": 30000
  }
}
```

### 3. Confirm the adapter argv before running

```bash
cd /Users/williamnewton/projects/basidiocarp/volva
cargo run -p volva-cli -- backend status
```

Expected lines:

```text
backend: official-cli
command: /bin/echo
hook_adapter: configured-external:cortina adapter volva hook-event
```

### 4. Run `volva` with a unique prompt marker

```bash
cd /Users/williamnewton/projects/basidiocarp/volva
cargo run -p volva-cli -- run volva-cortina-smoke-001
```

Expected stdout:

```text
-p [volva-host-context]
source: host-provided context from volva
cwd: /Users/williamnewton/projects/basidiocarp/volva
backend: official-cli
...
[user-prompt]
volva-cortina-smoke-001
```

The exact output now includes the host-provided `volva` context envelope before
the raw user prompt. The important proof points are that stdout starts with
`-p [volva-host-context]` and still contains the unique prompt marker under the
`[user-prompt]` section.

### 5. Confirm intake through supported `cortina` surfaces

```bash
cargo run --manifest-path /Users/williamnewton/projects/basidiocarp/cortina/Cargo.toml -- status --cwd /Users/williamnewton/projects/basidiocarp/volva
cargo run --manifest-path /Users/williamnewton/projects/basidiocarp/cortina/Cargo.toml -- doctor --cwd /Users/williamnewton/projects/basidiocarp/volva
cargo run -p volva-cli -- backend doctor
```

Expected `status` output includes:

```text
volva_hook_event_count=4
```

Expected `doctor` output includes:

```text
volva_hook_events_exists=true
volva_hook_events_valid_json=true
```

Expected `volva backend doctor` output includes:

```text
hook_delivery_probe: cortina-status-doctor
hook_delivery_seen_for_cwd: true
hook_delivery_event_count: 4
hook_delivery_events_valid_json: true
```

## Notes

- For a real operator configuration, switch `"backend.command"` back to
  `"claude"` after the smoke test.
- `cortina` records Volva hook events into temp-state JSON and surfaces the
  scoped count and file health through `status` and `doctor`.
