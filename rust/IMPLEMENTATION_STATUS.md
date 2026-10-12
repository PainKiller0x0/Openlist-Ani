# Implementation Status

This status records executable Rust behavior only. A copied acceptance case or a planned task is not a pass.

| Work area | Status | Evidence |
| --- | --- | --- |
| A. Compatibility foundation | Partial | Config loopback validation, health/readiness, authenticated private APIs, legacy auth contract tests, revision-checked settings writes |
| B. Data and pure rules | Partial | Legacy resource idempotent write/read, batched title/episode lookups, latest-episode exact/normalized fallback with season isolation, revision-checked atomic TOML writes preserving comments/unknown sections, memento raw-payload persistence tests; 14 naming golden cases |
| C. External adapters | Partial | OpenList endpoint/path checks plus offline-submit, task-list, file-list, rename, mkdir/move/remove contracts against local HTTP stubs; bounded RSS/Atom and `.torrent` fetch/parse endpoints; Mikan search/group HTML parsing and RSS URL construction, all covered by local HTTP contract tests. Mikan detail enrichment, TMDB/LLM/SmartStrm adapters remain absent |
| D. Download state machine and recovery | Partial | `/api/downloads` submits to OpenList and starts a persistent background worker; it polls download/transfer state, detects the completed video, persists a materialization plan, resolves filename conflicts, renames/moves the file, cleans the temp directory, and idempotently commits resource metadata. URL-hash deduplication, accepted/uncertain checkpoints, and startup resumption of confirmed submissions are implemented. A local HTTP+SQLite end-to-end test verifies submit-to-resource-commit. Reconciliation UI/API, full worker-restart fault injection, and retry/cancel controls remain absent |
| E. Torrent and magnet handling | Partial | Pure Rust bounded bencode parser, single-/multi-file listing, exact v1 info-hash checks against synthetic fixtures; magnet DHT metadata, raw torrent upload, and upload lifecycle remain absent |
| F. Browser/API compatibility | Partial | HTTP contracts cover auth, UI settings/state, download reads, Mikan search/groups/RSS, RSS preview/add/scan and subscription pause/complete/correct/exclusion/remove controls, RSS parsing, and torrent resolution. Metadata routes, torrent upload/magnet flow, and most of the 35 legacy routes and browser flows remain absent |
| G. Build, deployment, rollback, performance | Not started | No Linux release or live cutover |

Current local verification: `cargo +1.85.0 test --locked --all-targets` (63 tests), `cargo +1.85.0 clippy --locked --all-targets -- -D warnings`, and `cargo +1.85.0 fmt --all -- --check`. HTTP tests use local stub servers; persistence tests use temporary databases with the legacy schema. The Mikan web flow, RSS scan and subscription-management paths, plus download creation-to-resource-commit, pass locally against simulated services. Linux build, browser acceptance, full worker-restart fault injection, and production deployment remain unverified.
