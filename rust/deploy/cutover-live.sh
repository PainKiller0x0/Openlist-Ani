#!/bin/sh
set -eu
root=/opt/openlist-ani
backup=$(cat "$root/backups/rust-rollback-path")
binary="$root/rust-release/rust/target/release/openlist-ani-rs"
test -x "$binary"
curl -fsS http://127.0.0.1:27666/readyz >/dev/null
test -f "$backup/legacy-runtime.tar.gz"
mkdir -p "$root/bin" /etc/systemd/system/openlist-ani.service.d
install -m 755 "$binary" "$root/bin/openlist-ani-rs"
systemctl stop openlist-ani-rust-canary.service
systemctl stop openlist-ani.service
rollback_on_error() {
  rm -f /etc/systemd/system/openlist-ani.service.d/90-rust.conf
  systemctl daemon-reload
  systemctl restart openlist-ani.service
}
trap rollback_on_error EXIT
python3 "$root/rust-release/rust/deploy/live-check.py" final-snapshot
cat > /etc/systemd/system/openlist-ani.service.d/90-rust.conf <<'UNIT'
[Service]
ExecStart=
ExecStart=/opt/openlist-ani/bin/openlist-ani-rs /opt/openlist-ani/config.toml
Environment=OPENLIST_ANI_WEB_ROOT=/opt/openlist-ani/source/src/openlist_ani/adapters/inbound/http/web
Environment=OPENLIST_ANI_LOG_FILE=/opt/openlist-ani/logs/rust.log
Environment=OPENLIST_ANI_ALLOW_RESTART=1
Environment=OPENLIST_ANI_BACKGROUND_ENABLED=0
MemoryMax=192M
MemoryHigh=96M
UNIT
systemctl daemon-reload
systemctl start openlist-ani.service
ready=0
for attempt in 1 2 3 4 5 6 7 8 9 10; do
  if curl -fsS http://127.0.0.1:26666/readyz >/dev/null; then ready=1; break; fi
  sleep 1
done
test "$ready" = 1
sed -i 's/OPENLIST_ANI_BACKGROUND_ENABLED=0/OPENLIST_ANI_BACKGROUND_ENABLED=1/' /etc/systemd/system/openlist-ani.service.d/90-rust.conf
systemctl daemon-reload
systemctl restart openlist-ani.service
trap - EXIT
echo 'Rust is now serving production on the original port; rollback assets retained.'
