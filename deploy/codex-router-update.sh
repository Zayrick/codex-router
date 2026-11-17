#!/usr/bin/env bash
# 服务器端更新脚本，由 GitHub Actions 通过 SSH 调用：
#   sudo -n /usr/local/sbin/codex-router-update /tmp/codex-router-linux-amd64.tar.gz
# 流程：校验 → 备份二进制 → 停服 → 备份数据目录 → 替换二进制与前端 → 启动 → 检查，
# 失败时二进制与前端成对回滚。
# 前端安装在 WEB_DIR，配置文件需设置 [frontend] dir 指向它。
set -euo pipefail

SERVICE=codex-router
BIN=/usr/local/bin/codex-router
WEB_DIR=/usr/local/share/codex-router/web
DATA_DIR=/var/lib/codex-router
CONFIG=$DATA_DIR/config.toml
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

# 校验前端产物：入口存在、带 nonce 占位符、引用的资源齐全、不含符号链接
web="$pkg/web"
[[ -f $web/index.html ]] || { echo "发行包缺少 web/index.html" >&2; exit 1; }
grep -q '__CODEX_ROUTER_CSP_NONCE__' "$web/index.html" || { echo "web/index.html 缺少 CSP nonce 占位符" >&2; exit 1; }
[[ -z $(find "$web" -type l) ]] || { echo "web 目录包含符号链接" >&2; exit 1; }
while read -r asset; do
  [[ -f $web/${asset#/admin/} ]] || { echo "index.html 引用的资源不存在: $asset" >&2; exit 1; }
done < <(grep -o '/admin/assets/[^"]*' "$web/index.html")
if ! grep -Eq '^\[frontend\]' "$CONFIG" 2>/dev/null; then
  echo "警告: $CONFIG 未配置 [frontend] dir，控制台将返回 503。请添加：" >&2
  printf '  [frontend]\n  dir = "%s"\n' "$WEB_DIR" >&2
fi

# 先在旁边准备好新前端，停服后再整体替换
install -d -m 0755 "$(dirname "$WEB_DIR")"
rm -rf "$WEB_DIR.new"
cp -R "$web" "$WEB_DIR.new"
chown -R root:root "$WEB_DIR.new"
chmod -R u=rwX,go=rX "$WEB_DIR.new"

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
rm -rf "$WEB_DIR.bak"
[[ -d $WEB_DIR ]] && mv "$WEB_DIR" "$WEB_DIR.bak"
mv "$WEB_DIR.new" "$WEB_DIR"
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

# 控制台入口可访问（未配置前端或无法解析 bind 时跳过）。按实际监听地址探测，
# 通配地址改用对应的回环地址。
console_ok() {
  local bind host port
  command -v curl >/dev/null || return 0
  grep -Eq '^\[frontend\]' "$CONFIG" 2>/dev/null || return 0
  bind=$(awk -F'"' '/^[[:space:]]*bind[[:space:]]*=/{print $2; exit}' "$CONFIG")
  port=${bind##*:}
  host=${bind%:*}
  [[ $port =~ ^[0-9]+$ && -n $host ]] || return 0
  case $host in
    0.0.0.0) host=127.0.0.1 ;;
    "[::]") host="[::1]" ;;
  esac
  curl -fsS -o /dev/null --max-time 5 "http://$host:$port/admin"
}

if healthy && console_ok; then
  echo "更新成功: $new_version"
  exit 0
fi

echo "新版本启动失败，回滚二进制与前端" >&2
journalctl -u "$SERVICE" -n 50 --no-pager >&2 || true
systemctl stop "$SERVICE" || true
install -m 0755 "$BIN.bak" "$BIN"
if [[ -d $WEB_DIR.bak ]]; then
  rm -rf "$WEB_DIR"
  mv "$WEB_DIR.bak" "$WEB_DIR"
fi
systemctl start "$SERVICE"
echo "已回滚到 $old_version；如新版已迁移数据，可从 $backup_dir 恢复" >&2
exit 1
