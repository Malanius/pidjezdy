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
- Keep the CLI JSON schema version, the plugin's expected version, fixtures,
  tests, and README examples synchronized. When Rust and JavaScript implement
  the same presentation policy, cover the same boundary cases in both.

## Contribution workflow

- Work on one issue and pull request at a time. Do not begin the next issue
  until the user confirms that the current pull request was merged.
- Use conventional commit subjects and pull request titles. Prefer small,
  atomic commits that make the implementation sequence easy to review.
- Use descriptive Gitflow-style bookmark names, such as `feature/...`,
  `bugfix/...`, `hotfix/...`, or `release/...`; never name a bookmark after only
  a pull request number.
- Sign every AI-authored commit and every AI-authored pull request or issue body
  with an accurate `Co-authored-by: <agent or model> <email>` disclosure. Use
  the actual harness/model identity; never copy a different agent's identity.
- Compose multiline GitHub issue, pull request, review-reply, and comment bodies
  in files and pass those files to `gh`; do not embed escaped `\n` sequences.
- After opening a pull request, request a Copilot review. Inspect every review
  thread, including later review rounds, fix legitimate findings in atomic
  commits, reply to and resolve addressed threads, and request another review.
- Report a pull request as ready only when CI passes, Copilot recommends
  approval on the current head, no review threads remain unresolved, and the
  GitHub merge state is clean. The user performs the merge.
- After the user confirms a merge, run `jj git fetch` and create a fresh empty
  change on `main` with `jj new main`. The remote deletes merged bookmarks.

## Verification

Before handing off a change, run:

```console
cargo fmt --all -- --check
cargo build --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

When plugin files change, also run:

```console
node --check Model.js
node --test tests/model.test.js
omarchy plugin validate .
```
