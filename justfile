set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

# List the available recipes.
default:
    @just --list

# Install the current CLI and refresh its Bash and Zsh completions.
install-local: _install-cli install-bash-completions install-zsh-completions

[private]
_install-cli:
    cargo install --path crates/pidjezdy-cli --locked --force

# Atomically refresh the user-local Bash completion from this checkout.
install-bash-completions:
    scripts/install-completions.sh bash

# Atomically refresh the user-local Zsh completion from this checkout.
install-zsh-completions:
    scripts/install-completions.sh zsh
