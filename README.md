# pidjezdy

Actionable PID departure information for the terminal and, soon, the Omarchy
top bar.

The project is being built incrementally. The workspace currently provides
portable user configuration, departure ranking, and a defensive adapter for
PID's departure-board endpoint. CLI output and the Omarchy plugin will arrive
in focused follow-up pull requests.

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
