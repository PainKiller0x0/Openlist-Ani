# Implementation Status

This status records executable Rust behavior only. A copied acceptance case or a planned task is not a pass.

| Work area | Status | Evidence |
| --- | --- | --- |
| A. Compatibility foundation | Partial | Config loopback validation, health/readiness, authenticated private APIs, legacy auth contract tests, revision-checked settings writes |
| B. Data and pure rules | Partial | Legacy resource idempotent write/read, batched title/episode lookups, latest-episode exact/normalized fallback with season isolation, revision-checked atomic TOML writes preserving comments/unknown sections, memento raw-payload persistence tests; 14 naming golden cases |
| C. External adapters | Partial | OpenList endpoint/path checks against a local HTTP stub; bounded RSS/Atom and `.torrent` fetch/parse endpoints with local HTTP contract tests. Mikan detail enrichment, TMDB/LLM/SmartStrm adapters remain absent |
| D. Download state machine and recovery | Not started | No workflow implementation |
| E. Torrent and magnet handling | Partial | Pure Rust bounded bencode parser, single-/multi-file listing, exact v1 info-hash checks against synthetic fixtures; magnet DHT metadata, raw torrent upload, and upload lifecycle remain absent |
| F. Browser/API compatibility | Partial | HTTP contracts for auth, UI settings/state, download reads, RSS parsing, and torrent resolution; most of the 35 legacy routes and browser flows remain absent |
| G. Build, deployment, rollback, performance | Not started | No Linux release or live cutover |

Current local verification: `cargo +1.85.0 test --locked --all-targets` (38 tests) and `cargo +1.85.0 clippy --locked --all-targets -- -D warnings`. HTTP tests use local stub servers; persistence tests use temporary databases with the legacy schema. Linux build, browser acceptance, state-machine recovery, and deployment remain unverified.
