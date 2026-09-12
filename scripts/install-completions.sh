#!/usr/bin/env bash
set -euo pipefail

shell=${1:-}
data_home=${XDG_DATA_HOME:-${HOME:?HOME must be set}/.local/share}

case "$shell" in
  bash)
    completion_dir="$data_home/bash-completion/completions"
    completion_file="$completion_dir/pidjezdy"
    ;;
  zsh)
    completion_dir="$data_home/zsh/site-functions"
    completion_file="$completion_dir/_pidjezdy"
    ;;
  *)
    printf 'usage: %s {bash|zsh}\n' "${0##*/}" >&2
    exit 2
    ;;
esac

install -d "$completion_dir"
completion_tmp=$(mktemp "$completion_dir/.pidjezdy.XXXXXX")
trap 'rm -f "$completion_tmp"' EXIT

if [[ -n ${PIDJEZDY_COMPLETION_BIN:-} ]]; then
  "$PIDJEZDY_COMPLETION_BIN" completions "$shell" >"$completion_tmp"
else
  cargo run --quiet --locked --package pidjezdy -- completions "$shell" >"$completion_tmp"
fi

chmod 0644 "$completion_tmp"
mv -f "$completion_tmp" "$completion_file"
trap - EXIT
