# Implementation Status

This status records executable Rust behavior only. A copied acceptance case or a planned task is not a pass.

| Work area | Status | Evidence |
| --- | --- | --- |
| A. Compatibility foundation | Partial | Config loopback validation and health/readiness integration tests |
| B. Data and pure rules | Partial | Read-only legacy schema checks; 14 naming golden cases |
| C. External adapters | Not started | No adapter tests |
| D. Download state machine and recovery | Not started | No workflow implementation |
| E. Torrent and magnet handling | Not started | Synthetic fixtures copied; no parser/helper implementation |
| F. Browser/API compatibility | Not started | Static UI retained in the Python package; no Rust API contract tests |
| G. Build, deployment, rollback, performance | Not started | No Linux release or live cutover |

Current local verification: `cargo test --locked` and `cargo clippy --locked --all-targets -- -D warnings`.
