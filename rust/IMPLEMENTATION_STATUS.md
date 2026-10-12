# Implementation Status

This status records executable Rust behavior only. A copied acceptance case or a planned task is not a pass.

| Work area | Status | Evidence |
| --- | --- | --- |
| A. Compatibility foundation | Partial | Config loopback validation, health/readiness, authenticated private APIs, legacy auth contract tests, revision-checked settings writes |
| B. Data and pure rules | Partial | Legacy resource idempotent write/read, batched title/episode lookups, latest-episode exact/normalized fallback with season isolation, revision-checked atomic TOML writes preserving comments/unknown sections, memento raw-payload persistence tests; 14 naming golden cases |
| C. External adapters | Live core | Actual Mikan search/groups/RSS and explicit TMDB identity verified; OpenAI/Anthropic metadata and SmartStrm completion hook tested with local HTTP contracts; cloud null listings handled |
| D. Download state machine and recovery | Live core | Real OpenList submit/download/transfer/rename/move/DB commit verified with a 359,801,539-byte file; checkpoint resumed after restart/retry without resubmitting; failed-task archive and safe retry implemented; delayed move visibility covered |
| E. Torrent and magnet handling | Live core | Pure Rust bounded bencode parser and v1 hash; real HTTP torrent-to-magnet conversion, raw upload, legacy uploaded-file serving and display-name magnet resolution; DHT metadata remains absent |
| F. Browser/API compatibility | Live core | Six main pages verified in a real browser on the public domain; RSS controls, settings, logs, metadata, failed-task retry/archive and restart implemented |
| G. Build, deployment, rollback, performance | Live | Native Linux release replaces the original systemd executable; private backup and retained Python rollback runtime; state API p95 2.74 ms in 30 authenticated local requests |

Current local checks: 101 Rust tests, formatting, and Clippy with warnings denied. HTTP contracts use local stubs; live evidence includes actual Linux/OpenList/Thunder/Mikan/TMDB and browser checks. The existing port, Caddy route, authentication and configuration were retained; no parallel Python worker remains running. Set `OPENLIST_ANI_WEB_ROOT` outside the checkout.

Remaining full-contract gaps: magnet DHT metadata, Mikan account login/subscription, assistant/messaging integrations, automated TMDB completion checks, cancel/reconciliation UI, and exhaustive process-kill fault injection. Unsupported legacy checkpoints return a conflict rather than pretending to complete. The full original API inventory is not claimed as implemented. Deployment details and rollback command: `docs/LIVE_RELEASE.md`.
