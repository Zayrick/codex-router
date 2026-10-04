#!/usr/bin/env bash
# 服务器端更新脚本，由 GitHub Actions 通过 SSH 调用：
#   sudo -n /usr/local/sbin/codex-router-update /tmp/codex-router-linux-amd64.tar.gz
# 流程：校验 → 备份二进制 → 停服 → 备份数据目录 → 替换 → 启动 → 检查，失败自动回滚二进制。
set -euo pipefail

SERVICE=codex-router
BIN=/usr/local/bin/codex-router
DATA_DIR=/var/lib/codex-router
BACKUP_ROOT=/var/backups
KEEP_BACKUPS=5

archive=${1:?用法: codex-router-update <codex-router-linux-amd64.tar.gz>}
[[ $EUID -eq 0 ]] || { echo "需要 root 权限运行" >&2; exit 1; }
[[ -f $archive ]] || { echo "找不到文件: $archive" >&2; exit 1; }

work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT

tar -xzf "$archive" -C "$work_dir"
pkg="$work_dir/codex-router-linux-amd64"
(cd "$pkg" && sha256sum -c SHA256SUMS)
new_version=$("$pkg/codex-router" --version)
old_version=$("$BIN" --version 2>/dev/null || echo "未安装")
echo "当前版本: $old_version"
echo "新版本:   $new_version"

cp -p "$BIN" "$BIN.bak"
systemctl stop "$SERVICE"

backup_dir=$(mktemp -d "$BACKUP_ROOT/codex-router.XXXXXXXX")
if ! cp -a "$DATA_DIR" "$backup_dir/"; then
  echo "备份数据失败，恢复启动旧版本" >&2
  systemctl start "$SERVICE"
  exit 1
fi
echo "数据已备份到 $backup_dir"

# 只保留最近的若干份备份（备份包含账户凭据，避免长期堆积）
ls -dt "$BACKUP_ROOT"/codex-router.* 2>/dev/null | tail -n +$((KEEP_BACKUPS + 1)) | xargs -r rm -rf

install -m 0755 "$pkg/codex-router" "$BIN"
systemctl start "$SERVICE"

# 连续数秒保持 active 且主进程未变化，才认为启动成功（防止崩溃重启循环被误判）
healthy() {
  local pid
  sleep 2
  systemctl is-active --quiet "$SERVICE" || return 1
  pid=$(systemctl show -p MainPID --value "$SERVICE")
  for _ in 1 2 3 4 5; do
    sleep 1
    systemctl is-active --quiet "$SERVICE" || return 1
    [[ $(systemctl show -p MainPID --value "$SERVICE") == "$pid" ]] || return 1
  done
}

if healthy; then
  echo "更新成功: $new_version"
  exit 0
fi

echo "新版本启动失败，回滚二进制" >&2
journalctl -u "$SERVICE" -n 50 --no-pager >&2 || true
systemctl stop "$SERVICE" || true
install -m 0755 "$BIN.bak" "$BIN"
systemctl start "$SERVICE"
echo "已回滚到 $old_version；如新版已迁移数据，可从 $backup_dir 恢复" >&2
exit 1
