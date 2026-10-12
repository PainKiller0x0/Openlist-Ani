#!/bin/sh
set -eu
root=/opt/openlist-ani
mkdir -p "$root/rust-release"
tar -xzf /tmp/openlist-ani-rust.tgz -C "$root/rust-release"
python3 "$root/rust-release/rust/deploy/live-check.py" prepare
systemd-run --unit=openlist-ani-rust-build --collect \
  -p WorkingDirectory="$root/rust-release/rust" \
  -p MemoryMax=700M -p MemorySwapMax=1G -p CPUQuota=75% -p Nice=10 \
  /root/.cargo/bin/cargo build --locked --release --jobs 1
