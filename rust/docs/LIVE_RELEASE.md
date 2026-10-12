# VPS Rust Release - 2026-10-12

## Deployment

- Host: `150.158.121.88`; SSH port `2222`.
- Public UI: `https://op-ani.painkiller.top`.
- Existing service: `openlist-ani.service`; existing bind `127.0.0.1:26666` and Caddy retained.
- Executable: `/opt/openlist-ani/bin/openlist-ani-rs`.
- Source/build: `/opt/openlist-ani/rust-release/rust`.
- Overrides: `/etc/systemd/system/openlist-ani.service.d/90-rust.conf`.
- Private config/auth files remain under `/opt/openlist-ani`; no credentials in Git.
- Backup: `/opt/openlist-ani/backups/pre-rust-20261012T082427Z`.

The backup contains source/virtualenv archive, private config/auth copies, original service unit, online SQLite snapshots, hash manifest, and final stopped-service snapshots. Both databases passed integrity checks. Cutover refused if a legacy task was active.

## Acceptance

1. Canary on `127.0.0.1:27666`: authenticated web/API and 19 subscription identities/overrides matched Python.
2. Four actual active feeds parsed. JOJO `[04]` and the Wild Last Boss offset verified; preview directories corrected to match frozen download plans.
3. Actual Mikan search returned four results; selected work returned nine groups; RSS construction and TMDB `283938` matched the expected series.
4. A real torrent converted to a magnet, downloaded and transferred through Thunder. Restart/retry resumed the same remote task, then verified `Rust Acceptance - S01E01.mp4`, size `359801539`, and the resource commit. The isolated cloud directory was removed.
5. Browser checks passed tracking, downloads, logs, settings, add and JOJO details on the public domain. No API/page errors; JOJO filename season remained `1`, archive season `6`.
6. First production scan: 4 feeds, 19 entries, 1 submitted, 18 skipped, no feed errors. One old JOJO item remained subject to Thunder's preexisting copyright refusal. Failed URLs are excluded from further automatic scans; explicit retry remains available.
7. Config SHA-256 remained `e4af4fb2d4e08536d5de1f8b526696191682c25a6411f704c435acd7adae9f42`; both production SQLite integrity checks returned `ok`.
8. Final scanner revision: 4 feeds, 19 entries, 0 submitted, 19 skipped, no errors. Web restart returned successfully and systemd started a different healthy PID. This requested restart is distinct from a crash restart.

Initial memory snapshot: old service cgroup approximately 126.3 MiB; Rust cgroup approximately 4.9 MiB and process RSS approximately 13.3 MiB after UI/scan activity. This is a snapshot, not a sustained peak guarantee. Keep accounting methods separate. Local authenticated state API p95 was 2.74 ms across 30 requests.

## Rollback

```sh
sh /opt/openlist-ani/rust-release/rust/deploy/rollback-live.sh
```

Stops Rust, removes only its systemd drop-in and starts retained Python. Current compatible config/databases are retained; SQLite backups allow an explicitly chosen data restore. No live rollback was invoked after new Rust submissions; the script and cutover failure trap remain available.

Builds use one compiler job, `MemoryMax=700M`, `CPUQuota=75%` and reduced priority. Production uses `MemoryMax=192M`, `MemoryHigh=96M`, existing CPU cap, logs capped to two approximately 1 MiB files and a 1,000-line buffer.

The Rust branch runs Rust formatting/tests/Clippy/Linux release and deployment-helper lint. Other branches keep their existing Python checks. The frozen Python tree currently has one existing Ruff unused-variable failure and 32 Black formatting failures; it was not reformatted as part of this migration.
