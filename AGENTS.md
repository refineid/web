# Repository instructions

## Worktree policy

- Reading does not require a worktree; writing does. The main checkout is
  never edited directly. Before writing code or modifying files for any task,
  create a dedicated git worktree strictly under `../wt/`
  (e.g., `../wt/refineid-web-<topic>`) on a `<topic>` branch. Never
  work directly on `main` or modify files in the main checkout.
- All worktrees MUST live strictly under `../wt/`, never loose beside
  repositories or in parent directories.
- Never put a git worktree under `/tmp`. It is cleared on reboot and takes the
  branch's only checkout with it.
- One task, one worktree, one branch (`<topic>`), one pull request per branch.
- Merge the pull request after mandatory local verification and review pass,
  then remove the worktree and branch and fast-forward `main`.

## Record deferred findings

- While working, file a GitHub issue in the owning repository for each
  confirmed, actionable defect or quality gap that cannot reasonably be
  fixed within the current task. Filing these issues is authorized; do not
  wait for a separate instruction for each finding.
- Search existing open issues first. Reuse the matching issue and add only
  new, useful evidence instead of creating a duplicate. Group findings only
  when they share a cause and can be resolved by one focused change.
- State the observed behavior, expected behavior, affected repository-relative
  paths, reproduction or inspection evidence, impact, and acceptance checks.
  Distinguish observations from hypotheses and specification requirements.
  Never claim an unexecuted test or hardware operation was verified.
- Keep speculative improvements in working notes until they have a concrete
  problem and useful acceptance criteria. Avoid issue spam and severity claims
  unsupported by evidence.
- Never put credentials, PIN data or candidate lengths, card secrets,
  personal data, private workspace paths, or persistent device identifiers
  in issue text, logs, screenshots, attachments, or reproduction fixtures.
  Report security-sensitive findings through the repository's private
  reporting process; if no safe channel is available, notify the user
  without publishing sensitive details.
- An issue does not excuse a broken gate or incomplete work needed to make
  the current task correct. Fix findings required for the task before handing
  it over; file independently deferred work with a clear scope.
- Link newly filed or reused issues in the task handoff. If issue creation is
  unavailable, preserve a sanitized finding locally and report that it was
  not filed; never silently discard it.

## Scope

- Home repo is `refineid/web`. Sibling checkouts (`refineid-core`,
  `refineid-unix`, `refineid-android`, `refineid-apple`, ...) are the
  protocol and platform owners: read them freely, change them only when the
  active plan names them, one repo per commit series.
- Never touch third-party login ceremonies (Suomi.fi, Telia flows). Adapt
  to them; existing RefineID integrations there are out of scope.
- No card operations from this repo's prototypes without explicit user
  direction. Never record PINs, PUKs, CANs, card data, USB serials, or
  persistent device IDs. Test PKI only, always git-ignored.

## Style (sniffed from sibling repos, proto phase)

- One-line header on every source file, comment syntax to match:
  `Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.`
  No headers in Markdown docs. Keep the repo LICENSE file as-is.
- Sources are ASCII-only: no smart quotes, em dashes, or ellipsis
  characters in code. Markdown prose may use normal typography.
- Width 100 everywhere (matches sibling `rustfmt.toml`).
- Formatters run before every commit, no exceptions:
  - JS/HTML: `npx prettier --check .` (config: `.prettierrc`)
  - Python: `.venv/bin/ruff check` + `.venv/bin/ruff format --check` (config: `pyproject.toml`)
  - Shell: `shellcheck -S warning`
  - Rust (when touched): `cargo fmt --check`, `cargo clippy`
- Working code beats strictness in this phase: proto-grade lint sets are
  intentional. Tighten only when a class of bug earns it.

## Verification

- Run `scripts/verify.sh` before handing over a change; it runs every
  format/lint check and test suite. Its prerequisites are listed in
  `README.md` under "Automated tests".
