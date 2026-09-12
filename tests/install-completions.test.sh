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
export PIDJEZDY_COMPLETION_BIN="$success_bin"
scripts/install-completions.sh zsh

completion="$XDG_DATA_HOME/zsh/site-functions/_pidjezdy"
[[ $(<"$completion") == "generated for zsh" ]]

printf 'existing completion\n' >"$completion"
export PIDJEZDY_COMPLETION_BIN="$failure_bin"
if scripts/install-completions.sh zsh; then
  printf 'expected completion generation to fail\n' >&2
  exit 1
fi

[[ $(<"$completion") == "existing completion" ]]
if compgen -G "$XDG_DATA_HOME/zsh/site-functions/.pidjezdy.*" >/dev/null; then
  printf 'temporary completion file was not removed\n' >&2
  exit 1
fi
