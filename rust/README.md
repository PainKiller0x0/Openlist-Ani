# OpenList-Ani Rust Migration

This directory contains the Rust backend and its implementation contract. The VPS switched its existing `openlist-ani.service` to this binary on 2026-10-12 after an isolated live download and browser acceptance. The previous Python runtime remains available for rollback.

## Current implementation

- Reads the existing `[backend]` host and port without rewriting TOML, and rejects non-loopback bind addresses.
- Provides `/healthz` and `/readyz`. Readiness opens the two legacy SQLite files read-only and checks their expected tables and columns; it never creates or migrates them.
- Implements the legacy PBKDF2-SHA256 password hash and stateless HMAC session cookie for login/session/logout; startup refuses missing or malformed auth settings.
- Reads and writes the existing SQLite `resources` and `task_mementos` tables; Rust download reservations use an additional table in the task database. Resource inserts are idempotent.
- Builds a naming plan with independent identity, filename, and directory seasons. The independent 14-case golden fixture is exercised by an integration test.

- Serves the existing web UI, Mikan search/groups, RSS preview and subscription controls, logs, settings, and controlled service restart.
- Converts torrent URLs and raw uploaded torrent bytes to magnets before submitting to OpenList. Magnet display names are supported; DHT metadata resolution is absent.
- Runs persistent download workers and RSS scanning, preserves naming overrides, waits for cloud rename/move visibility, and notifies the configured SmartStrm relay after completion.
- Resolves metadata through TMDB, trusted Mikan aliases, and the configured OpenAI/Anthropic endpoint. Ambiguous matches fail explicitly; saved naming and season overrides survive refresh.
- Supports failed-task archive and safe checkpoint retry. Uncertain submissions and unsupported legacy checkpoints require reconciliation rather than automatic resubmission.

The live acceptance downloaded, transferred, renamed, and verified a 359,801,539-byte video in an isolated OpenList directory, then removed that directory. All six main UI pages passed browser checks on the public domain. The original configuration SHA-256 and 19 subscriptions were preserved. See `IMPLEMENTATION_STATUS.md` for remaining migration gaps and `docs/LIVE_RELEASE.md` for deployment evidence.

## Local checks

From this directory:

```text
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

The complete requirements, API contract, test plan, deployment runbook, acceptance matrix, and synthetic fixtures are kept alongside this crate under `docs/`, `fixtures/`, and `reference/`.

## Live deployment tools

The `deploy/` scripts stage a Linux build with bounded CPU/memory, back up the runtime and SQLite files, start a separate authenticated canary, verify real adapters/downloads, and switch the original service through a systemd drop-in. `rollback-live.sh` restores the retained Python executable without replacing current compatible data. Credentials remain in the existing private files; they are not deployment artifacts in Git.
