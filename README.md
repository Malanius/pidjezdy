# pidjezdy

Actionable PID departure information for the terminal and the Omarchy top bar.

The project is being built incrementally. The workspace currently provides a
working CLI with portable user configuration, departure ranking, and a
defensive adapter for PID's departure-board endpoint. It retains an atomic
local fallback when PID is temporarily unavailable, while the Omarchy plugin
polls that CLI and presents its results in a native bar popup.

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

- `display.max_departures`: overall number of results to show, from 1 to 20.
- `display.route_quotas`: optional minimum numbers of departures to reserve for
  particular line and direction pairs before filling the remaining result
  slots chronologically.
- `fetch.minutes_after`: future window requested from PID, in minutes.
- `fetch.api_limit`: result limit applied independently to each configured
  boarding point, from 1 to 20.
- `boarding_points`: one entry per place and walking route you could use. Each
  entry has a display `name`, one or more PID platform `stop_ids`, walking
  time, safety buffer, and accepted line/headsign pairs.

For example:

```toml
[display]
max_departures = 4

[[display.route_quotas]]
line = "123"
headsign = "City centre"
minimum_departures = 2

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

Human-readable output uses aligned two-line records separated at the full
rendered width. It rounds time down conservatively:

```text
123  City centre                  leave in 4 min
     Nearby stop · platform A  departs in 10 min
────────────────────────────────────────────────
456  Main station                leave in 7 min
     Other stop · platform C   departs in 15 min
```

On an interactive terminal, the line, direction, and leave-by time are
emphasized while secondary details and separators are dimmed. ANSI styling is
automatically omitted when output is redirected or piped, and can also be
disabled by setting [`NO_COLOR`](https://no-color.org/). JSON output never
contains terminal styling.

The selector first reserves up to each route's configured minimum, when that
many matching departures are available, and then fills unused result slots
with the nearest departures overall. A quota matches the normalized `line` and
`headsign` across all boarding points, so one physical trip appearing at
multiple configured stops remains a single result. Route minimums must refer
to configured boarding-point routes, and their sum cannot exceed
`display.max_departures`.

Surrounding whitespace is removed from configured names, stop IDs, lines, and
headsigns when the file is loaded. PID identifiers and display text are
normalized the same way before selection, so matching and cache identity use
one consistent representation. Library callers that construct `Config`
directly should call `Config::normalize` before selection; `Config::from_toml`
does this automatically.

Override the configured count for one invocation with `--limit`:

```console
pidjezdy departures --limit 2
```

The command-line limit is a hard ceiling even when it is smaller than the
configured route minimums. Constrained slots are allocated in rounds: the
nearest departure from each quota route is considered before a second
departure from any route. The selected results are then printed in departure
order. This makes a small limit useful for compact consumers without allowing
one frequent route to take every slot.

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

Successful JSON output has this envelope:

```json
{
  "schema_version": 1,
  "generated_at": "2026-09-05T08:00:00Z",
  "data_updated_at": "2026-09-05T07:59:30Z",
  "stale": false,
  "departures": []
}
```

Relative departure times are included as exact seconds so consumers do not
need to infer them from rounded labels. When the command fails after accepting
`--format json`, it writes one error document to stdout and exits non-zero:

```json
{
  "schema_version": 1,
  "generated_at": "2026-09-05T08:00:00Z",
  "error": {
    "kind": "departures_unavailable",
    "message": "departures unavailable",
    "causes": ["could not fetch PID departures", "connection refused"]
  }
}
```

`error.kind` is a stable machine-readable value. Departure queries can report
`config_directory_unavailable`, `config_unreadable`, `config_invalid`,
`request_invalid`, `departures_unavailable`, `diagnostics_write_failed`, or
`output_failed`. The human-readable `message` and `causes` may become more
detailed over time and should not be parsed for control flow. Errors that
prevent writing stdout itself cannot produce a JSON document.

Consumers must require the supported `schema_version`. The version is bumped
when a field is removed, renamed, retyped, or changes meaning without changing
its name. Purely additive fields do not require a bump.

## Shell completions

Generate completion scripts for Bash, Elvish, Fish, PowerShell, or Zsh with:

```console
pidjezdy completions <shell>
```

Load Bash completions for the current session with:

```bash
source <(pidjezdy completions bash)
```

For persistent completion, write the generated script to the location your
shell scans. Common user-local locations are:

```bash
# Bash (with bash-completion installed)
mkdir -p ~/.local/share/bash-completion/completions
pidjezdy completions bash > ~/.local/share/bash-completion/completions/pidjezdy

# Fish
mkdir -p ~/.config/fish/completions
pidjezdy completions fish > ~/.config/fish/completions/pidjezdy.fish

# Zsh; add this directory to fpath before running compinit
mkdir -p ~/.local/share/zsh/site-functions
pidjezdy completions zsh > ~/.local/share/zsh/site-functions/_pidjezdy
```

PowerShell can load completions for the current session with:

```powershell
pidjezdy completions powershell | Out-String | Invoke-Expression
```

Add that command to `$PROFILE` to load it in future PowerShell sessions.

## Cache and stale data

After every successful PID request containing at least one departure,
`pidjezdy` atomically replaces a snapshot in the platform-standard cache
directory. The snapshot contains the raw, normalized departures, their fetch
time, and the PID request that produced them: the time window, API limit, and
grouped stop IDs. A cache-write failure does not hide fresh results; it is
reported as a warning on stderr.

A successful empty response is still rendered as fresh data, but it does not
replace an existing non-empty snapshot. This protects the fallback from brief
empty responses during provider degradation. If a later request fails, cached
departures are selected again against the current time, so expired services are
not resurrected.

If PID cannot be reached or its response cannot be decoded, the CLI reads that
snapshot only when its request exactly matches the current one. Cached raw
departures are filtered and ranked again using the current time, so trips that
are no longer reachable disappear normally. Changing presentation and
selection settings—such as stop names, walking times, safety buffers, routes,
quotas, or the display limit—does not invalidate the snapshot because those
rules are reapplied when it is read.

Cached output is always labeled. Text output starts with a line such as:

```text
STALE · data updated 3 min ago
```

JSON sets `stale` to `true`, preserves the original fetch time in
`data_updated_at`, and uses the current processing time for `generated_at`.
There is intentionally no hidden age threshold: consumers can use the explicit
timestamps to choose their own policy, while departures naturally age out of
the configured future window. A cache with an unsupported format, malformed
content, or a different request is rejected instead of being shown.

In text mode, fatal diagnostics keep the first stderr line concise, then print
each distinct underlying cause on an indented line. PID transport errors retain
useful details such as timeouts or refused connections, while request URLs and
configured stop IDs are omitted from the default report.

## Omarchy plugin

The repository is also an Omarchy `bar-widget` plugin. Its bus icon opens a
native popup containing the closest reachable departures selected by the CLI.
It defaults to the right side of the bar, polls once per minute, and requests
three departures so the popup remains compact. The plugin does not run a
daemon or access PID directly.

The plugin requests JSON and validates the shared envelope version for both
successful and failed refreshes. It keeps the last successful departures
visible when a later refresh fails.

The CLI must be installed and configured first. When installing from the Git
repository:

```bash
cargo install --git https://github.com/Malanius/pidjezdy --locked pidjezdy
pidjezdy config init
```

Edit the resolved configuration and check it as described above, then install
and enable the plugin:

```bash
omarchy plugin add https://github.com/Malanius/pidjezdy.git --enable --yes
```

`malanius.pidjezdy` declares `right` as its default section. It can still be
moved like any other Omarchy widget:

```bash
omarchy bar move malanius.pidjezdy --section right
```

Left-click the icon to toggle the popup. Middle-click, `Enter`, or `r` refreshes
immediately; `Esc` closes it. The popup advances the CLI's exact countdowns
between polls and removes a departure as soon as its leave-by time passes, so
the displayed minutes never overpromise. Failed refreshes retain the previous
successful result but label the failure, while cached CLI results keep their
`STALE` label.

The plugin exposes two Omarchy settings:

- `refreshIntervalSec`: polling interval from 30 to 3600 seconds; defaults to
  60.
- `limit`: hard CLI result limit from 1 to 20; defaults to 3. Route quota slots
  are allocated fairly when this is below the configured minimum total.

They can be changed through Omarchy's plugin settings UI or from the command
line:

```bash
omarchy bar set malanius.pidjezdy refreshIntervalSec 60
omarchy bar set malanius.pidjezdy limit 3
```

## Development

This repository uses colocated [Jujutsu](https://jj-vcs.github.io/jj/) and Git
metadata. Prefer `jj status`, `jj diff`, and `jj log` during development.

Run the project checks with:

```console
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
node --check Model.js
node --test tests/model.test.js
omarchy plugin validate .
```

For local plugin development, link the checkout into the user plugin
directory. Omarchy hot-reloads changes:

```bash
ln -s "$PWD" ~/.config/omarchy/plugins/malanius.pidjezdy
omarchy-shell shell rescanPlugins
omarchy plugin enable malanius.pidjezdy
```

## License

[MIT](LICENSE)
