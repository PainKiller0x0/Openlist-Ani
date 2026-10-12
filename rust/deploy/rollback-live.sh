#!/bin/sh
set -eu
root=/opt/openlist-ani
test -f "$root/backups/rust-rollback-path"
test -x "$root/venv/bin/openlist-ani"
systemctl stop openlist-ani.service
rm -f /etc/systemd/system/openlist-ani.service.d/90-rust.conf
systemctl daemon-reload
systemctl start openlist-ani.service
echo 'Legacy Python runtime restored; current compatible data and configuration retained.'
