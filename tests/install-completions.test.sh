#!/usr/bin/env bash
set -euo pipefail

test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT

success_bin="$test_root/pidjezdy-success"
failure_bin="$test_root/pidjezdy-failure"

printf '#!/usr/bin/env bash\nprintf "generated for %%s\\n" "$2"\n' >"$success_bin"
printf '#!/usr/bin/env bash\nprintf "partial output\\n"\nexit 1\n' >"$failure_bin"
chmod +x "$success_bin" "$failure_bin"

export XDG_DATA_HOME="$test_root/data"

check_installation() {
  local shell=$1
  local completion=$2

  export PIDJEZDY_COMPLETION_BIN="$success_bin"
  scripts/install-completions.sh "$shell"
  [[ $(<"$completion") == "generated for $shell" ]]

  printf 'existing completion\n' >"$completion"
  export PIDJEZDY_COMPLETION_BIN="$failure_bin"
  if scripts/install-completions.sh "$shell"; then
    printf 'expected %s completion generation to fail\n' "$shell" >&2
    exit 1
  fi

  [[ $(<"$completion") == "existing completion" ]]
  if compgen -G "${completion%/*}/.pidjezdy.*" >/dev/null; then
    printf 'temporary %s completion file was not removed\n' "$shell" >&2
    exit 1
  fi
}

check_installation bash "$XDG_DATA_HOME/bash-completion/completions/pidjezdy"
check_installation zsh "$XDG_DATA_HOME/zsh/site-functions/_pidjezdy"
