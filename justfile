set shell := ["bash", "-euo", "pipefail", "-c"]

# List the available recipes.
default:
    @just --list

# Install the current CLI and refresh its Bash completion.
install-local: _install-cli install-bash-completions

[private]
_install-cli:
    cargo install --path crates/pidjezdy-cli --locked --force

# Atomically refresh the user-local Bash completion from this checkout.
install-bash-completions:
    #!/usr/bin/env bash
    set -euo pipefail

    data_home="${XDG_DATA_HOME:-${HOME:?HOME must be set}/.local/share}"
    completion_dir="$data_home/bash-completion/completions"
    install -d "$completion_dir"

    completion_tmp="$(mktemp "$completion_dir/.pidjezdy.XXXXXX")"
    trap 'rm -f "$completion_tmp"' EXIT

    cargo run --quiet --locked --package pidjezdy -- completions bash > "$completion_tmp"
    chmod 0644 "$completion_tmp"
    mv -f "$completion_tmp" "$completion_dir/pidjezdy"
    trap - EXIT
