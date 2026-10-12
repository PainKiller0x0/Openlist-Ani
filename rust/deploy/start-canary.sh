#!/bin/sh
set -eu
root=/opt/openlist-ani
systemctl stop openlist-ani-rust-canary.service 2>/dev/null || true
systemctl reset-failed openlist-ani-rust-canary.service 2>/dev/null || true
systemd-run --unit=openlist-ani-rust-canary --collect \
  -p WorkingDirectory="$root/rust-canary" \
  -p EnvironmentFile="$root/auth.env" -p MemoryMax=192M \
  --setenv=OPENLIST_ANI_BACKGROUND_ENABLED=0 \
  --setenv=OPENLIST_ANI_RECOVER_DOWNLOADS_ENABLED=1 \
  --setenv=OPENLIST_ANI_WEB_ROOT="$root/source/src/openlist_ani/adapters/inbound/http/web" \
  --setenv=OPENLIST_ANI_LOG_FILE="$root/rust-canary/rust.log" \
  "$root/rust-release/rust/target/release/openlist-ani-rs" "$root/rust-canary/config.toml"
