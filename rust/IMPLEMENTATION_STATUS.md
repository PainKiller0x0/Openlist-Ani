# Implementation Status

This status records executable Rust behavior only. A copied acceptance case or a planned task is not a pass.

| Work area | Status | Evidence |
| --- | --- | --- |
| A. Compatibility foundation | Partial | Config loopback validation, health/readiness, authenticated private APIs, legacy auth contract tests, revision-checked settings writes |
| B. Data and pure rules | Partial | Legacy resource idempotent write/read, batched title/episode lookups, latest-episode exact/normalized fallback with season isolation, revision-checked atomic TOML writes preserving comments/unknown sections, memento raw-payload persistence tests; 14 naming golden cases |
| C. External adapters | Partial | OpenList endpoint/path checks plus offline-submit, task-list, file-list, rename, mkdir/move/remove contracts against local HTTP stubs; bounded RSS/Atom and `.torrent` fetch/parse endpoints with local HTTP contract tests. Mikan detail enrichment, TMDB/LLM/SmartStrm adapters remain absent |
| D. Download state machine and recovery | Partial | SQLite submission-intent reservation, URL-hash deduplication, accepted/uncertain checkpoints, restart conversion from `sending` to `uncertain`, and a tested helper for recursive largest-video detection, conflict-safe rename, materialization, and temp cleanup. Still no persistent end-to-end worker, reconciliation UI/API, resource commit, or automatic startup recovery integration |
| E. Torrent and magnet handling | Partial | Pure Rust bounded bencode parser, single-/multi-file listing, exact v1 info-hash checks against synthetic fixtures; magnet DHT metadata, raw torrent upload, and upload lifecycle remain absent |
| F. Browser/API compatibility | Partial | HTTP contracts for auth, UI settings/state, download reads, RSS parsing, and torrent resolution; most of the 35 legacy routes and browser flows remain absent |
| G. Build, deployment, rollback, performance | Not started | No Linux release or live cutover |

Current local verification: `cargo +1.85.0 test --locked --all-targets` (45 tests) and `cargo +1.85.0 clippy --locked --all-targets -- -D warnings`. HTTP tests use local stub servers; persistence tests use temporary databases with the legacy schema. Linux build, browser acceptance, end-to-end worker recovery, and deployment remain unverified.
