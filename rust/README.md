# OpenList-Ani Rust Migration

This directory contains the Rust replacement and its implementation contract. The Python service remains the only production implementation while this migration is in progress.

## Current implementation

- Reads the existing `[backend]` host and port without rewriting TOML, and rejects non-loopback bind addresses.
- Provides `/healthz` and `/readyz`. Readiness opens the two legacy SQLite files read-only and checks their expected tables and columns; it never creates or migrates them.
- Implements the legacy PBKDF2-SHA256 password hash and stateless HMAC session cookie for login/session/logout; startup refuses missing or malformed auth settings.
- Builds a naming plan with independent identity, filename, and directory seasons. The independent 14-case golden fixture is exercised by an integration test.

Authentication, business APIs, RSS, external integrations, persistent downloads, browser compatibility, and deployment are not implemented yet. Do not use this binary as a production replacement or run it against the live service port.

## Local checks

From this directory:

```text
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

The complete requirements, API contract, test plan, deployment runbook, acceptance matrix, and synthetic fixtures are kept alongside this crate under `docs/`, `fixtures/`, and `reference/`.
