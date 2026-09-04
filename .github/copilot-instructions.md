# Copilot review guidance

Review this repository with special attention to:

- configuration precedence and accidental project-local or personal data;
- exact reachability calculations and conservative display rounding;
- tolerant handling of undocumented PID responses, nulls, and compression;
- atomic cache updates and unambiguous stale-state reporting;
- subprocess overlap and lifecycle safety in the future QuickShell plugin.

Call out behavior that is not covered by a deterministic test. Do not suggest
coupling provider response types directly to CLI or QML presentation models.

