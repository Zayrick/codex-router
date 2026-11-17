#!/bin/sh
set -eu
umask 077

test "$(id -u)" = 0
backup=$(cat /etc/codex-router/rollback-dir)
case "$backup" in /var/backups/codex-router-migration/*) ;; *) echo "Invalid backup path" >&2; exit 1 ;; esac
test -f "$backup/legacy.tar.gz"
(cd "$backup" && sha256sum -c SHA256SUMS)
exec 9>/run/codex-router-migration.lock
flock -n 9

save="/var/backups/codex-router-before-rollback/$(date +%Y%m%dT%H%M%S)"
mkdir -p "$save"
rc-service codex-router-bootstrap stop
rc-service codex-router stop
cp -a /var/lib/codex-router /var/lib/codex-router-bootstrap "$save/"
cp -a /etc/cliproxyapi /etc/cpa-usage-keeper /var/lib/cliproxyapi/auths /var/lib/codex-bootstrap "$save/"

if [ "${1:-}" = "--snapshot" ]; then
    tar -xzf "$backup/legacy.tar.gz" -C /
else
    python3 /opt/codex-router/current/deploy/rollback_sync.py
fi
if [ -f /etc/nginx/http.d/codex-router-console.conf ]; then
    python3 /opt/codex-router/current/deploy/rollback_sync.py --configure-lan-listener
fi
cp -a "$backup/crontab" /etc/crontabs/root
if [ -f /etc/nginx/http.d/codex-router-compat.conf ]; then
    mv /etc/nginx/http.d/codex-router-compat.conf "$save/"
fi
nginx -t
rc-service nginx reload
for service in codex-router-bootstrap codex-router; do
    rc-update del "$service" default
done
for service in cliproxyapi cpa-usage-keeper codex-account-proxy codex-bootstrap; do
    rc-update add "$service" default
    rc-service "$service" start
done
echo "CLIProxyAPI stack restored. Router state saved at $save"
