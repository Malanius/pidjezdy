# Repository guidance

- Use colocated Jujutsu for source-control work. Prefer `jj status`, `jj diff`,
  bookmarks, and `jj git push` over direct Git commands.
- Keep provider DTOs in `pidjezdy-pid`, user-facing configuration and departure
  selection in `pidjezdy-core`, and process/filesystem/UI orchestration in the
  `pidjezdy` CLI or Omarchy plugin.
- Keep personal stop IDs and walking times out of tracked examples and tests.
- Treat the PID `data.php` endpoint as undocumented. Parse defensively and back
  observed behavior with captured fixtures and `docs/pid-api.md`.
- Calculate reachability with exact timestamps. Round only at presentation
  boundaries, conservatively, so displayed time never overpromises.
- Cache writes must be atomic and stale data must always be labeled.
- Before handing off a change, run formatting, Clippy with warnings denied, and
  all workspace tests. Validate the Omarchy manifest when plugin files change.

