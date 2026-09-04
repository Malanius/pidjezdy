# pidjezdy

Actionable PID departure information for the terminal and, soon, the Omarchy
top bar.

The project is being built incrementally. The workspace currently provides a
working CLI with portable user configuration, departure ranking, and a
defensive adapter for PID's departure-board endpoint. It retains an atomic
local fallback when PID is temporarily unavailable. The Omarchy plugin will
arrive in a focused follow-up pull request.

The departure endpoint is not officially specified. Its observed contract is
documented in [`docs/pid-api.md`](docs/pid-api.md).

## Configuration

`pidjezdy` follows platform conventions instead of reading configuration from
the project directory. Resolve the active location with:

```console
pidjezdy config path
```

Resolution order is:

1. `--config <path>`
2. `PIDJEZDY_CONFIG`
3. the platform-standard configuration directory

Empty CLI or environment overrides are treated as unset and fall through to
the next source.

Create a commented starter configuration with `pidjezdy config init`, then
validate it with `pidjezdy config check`. Initialization never overwrites an
existing file.

The main settings are:

- `display.max_departures`: default number of results to show, from 1 to 20.
- `fetch.minutes_after`: future window requested from PID, in minutes.
- `fetch.api_limit`: result limit applied independently to each configured
  boarding point, from 1 to 20.
- `boarding_points`: one entry per place and walking route you could use. Each
  entry has a display `name`, one or more PID platform `stop_ids`, walking
  time, safety buffer, and accepted line/headsign pairs.

For example:

```toml
[display]
max_departures = 3

[fetch]
minutes_after = 120
api_limit = 20

[[boarding_points]]
name = "Nearby stop"
stop_ids = ["U123Z1P", "U123Z2P"]
walking_minutes = 4
safety_buffer_minutes = 2

[[boarding_points.routes]]
line = "123"
headsign = "City centre"
```

Line and headsign matching is exact after surrounding whitespace is removed.
Keep separate boarding-point entries when platforms have different walking
times, even if they share a passenger-facing stop name.

## Departure queries

Show the configured number of reachable departures with:

```console
pidjezdy departures
```

Text output is compact and rounds time down conservatively:

```text
123 → City centre · Nearby stop · platform A · leave in 4 min · departs in 10 min
```

Override the configured count for one invocation with `--limit`:

```console
pidjezdy departures --limit 2
```

A departure is reachable when its predicted time, or scheduled time when no
prediction exists, leaves at least the configured walking time plus safety
buffer. Cancelled, unmatched, and already-unreachable departures are omitted.
When the same trip serves multiple configured boarding points, the CLI keeps
the option that leaves the most time to reach it.

For scripts and the future Omarchy plugin, request JSON instead of parsing the
human-readable text:

```console
pidjezdy departures --format json
```

The JSON document contains `generated_at`, `data_updated_at`, a `stale` flag,
and a `departures` array. Relative times are also included as exact seconds so
consumers do not need to infer them from rounded labels.

## Cache and stale data

After every successful PID request, `pidjezdy` atomically replaces a snapshot
in the platform-standard cache directory. The snapshot contains the raw,
normalized departures, their fetch time, and the validated configuration that
produced the request. A cache-write failure does not hide fresh results; it is
reported as a warning on stderr.

If PID cannot be reached or its response cannot be decoded, the CLI reads that
snapshot only when its configuration exactly matches the current one. Cached
raw departures are filtered and ranked again using the current time, so trips
that are no longer reachable disappear normally.

Cached output is always labeled. Text output starts with a line such as:

```text
STALE · data updated 3 min ago
```

JSON sets `stale` to `true`, preserves the original fetch time in
`data_updated_at`, and uses the current processing time for `generated_at`.
There is intentionally no hidden age threshold: consumers can use the explicit
timestamps to choose their own policy, while departures naturally age out of
the configured future window. A cache with an unsupported format, malformed
content, or a different configuration is rejected instead of being shown.

## Development

This repository uses colocated [Jujutsu](https://jj-vcs.github.io/jj/) and Git
metadata. Prefer `jj status`, `jj diff`, and `jj log` during development.

Run the project checks with:

```console
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

[MIT](LICENSE)
