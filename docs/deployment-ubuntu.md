# Ubuntu 部署与更新

适用于 Ubuntu 22.04 或更高版本的 x86_64（amd64）服务器。GitHub Actions 的
`Build Linux amd64` 工作流生成 `codex-router-linux-amd64.tar.gz`，包含二进制、配置模板和校验文件。
前端已嵌入二进制，服务器无需安装 Rust、Node.js 或 pnpm。ARM 服务器不能使用此包。

## 1. 上传并解压

在本机执行，将用户名和服务器 IP 替换为实际值：

```sh
scp codex-router-linux-amd64.tar.gz 用户名@服务器IP:/tmp/
```

以下命令均在登录 Ubuntu 服务器后执行。如果下载的是 ZIP，先解压得到上述 `.tar.gz` 文件。

```sh
cd /tmp
tar -xzf codex-router-linux-amd64.tar.gz
cd codex-router-linux-amd64
sha256sum -c SHA256SUMS
./codex-router --version
```

校验成功、版本命令正常退出后继续。

## 2. 首次安装

创建专用服务用户，安装程序和配置。以下配置复制命令仅用于首次安装：

```sh
sudo useradd --system --home-dir /var/lib/codex-router \
  --shell /usr/sbin/nologin codex-router
sudo install -m 0755 codex-router /usr/local/bin/codex-router
sudo install -d -m 0750 -o codex-router -g codex-router /var/lib/codex-router
sudo install -m 0600 -o codex-router -g codex-router \
  config.example.toml /var/lib/codex-router/config.toml
sudo nano /var/lib/codex-router/config.toml
```

修改 `admin.path`、`admin.secret` 和 `state.api_keys` 中的默认 Key。使用同机 HTTPS 反向代理时，
保留 `server.bind = "127.0.0.1:8787"`，将 `server.public_origin` 改成实际访问地址，
例如 `"https://router.example.com"`（不带路径或尾部斜杠）。

默认直接连接 ChatGPT。如需 SOCKS5 出站代理，在 `[upstream]` 中配置 `chatgpt_proxy`；
详细设置见[配置文档](configuration.md)。

服务用户需要对整个 `/var/lib/codex-router` 目录有写权限，用于原子更新配置和写入 SQLite。
默认数据库位于 `/var/lib/codex-router/usage.sqlite3`。

## 3. 创建 systemd 服务

```sh
sudo nano /etc/systemd/system/codex-router.service
```

粘贴并保存：

```ini
[Unit]
Description=Codex Router
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=codex-router
Group=codex-router
WorkingDirectory=/var/lib/codex-router
ExecStart=/usr/local/bin/codex-router --config /var/lib/codex-router/config.toml
Restart=on-failure
RestartSec=3
UMask=0077

[Install]
WantedBy=multi-user.target
```

启动并设置开机启动：

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now codex-router
sudo systemctl status codex-router --no-pager
```

查看实时日志：

```sh
sudo journalctl -u codex-router -f
```

## 4. 访问管理页面

通过 Nginx 或 Caddy 将 HTTPS 请求代理到 `127.0.0.1:8787`，允许 WebSocket Upgrade，
关闭 SSE 响应缓冲。管理 Cookie 带有 `Secure`，通过公网 HTTP 访问不能正常维持管理登录。
同机反向代理部署无需对公网开放 8787 端口。

访问 `https://你的域名/你的admin.path/admin`，使用 `admin.secret` 登录，
添加至少一个 Codex 账户，并将 API Key 分配到账户或账户组。

## 5. 后续更新二进制

每次下载新版后，重复第 1 步上传、解压、校验，并确认版本命令正常退出。然后执行：

```sh
cd /tmp/codex-router-linux-amd64

# 备份旧二进制，然后停止服务
sudo cp -p /usr/local/bin/codex-router /usr/local/bin/codex-router.bak && \
sudo systemctl stop codex-router

# 停服后备份配置和数据库；备份目录仅 root 可访问
backup_dir=$(sudo mktemp -d /var/backups/codex-router.XXXXXXXX) && \
sudo cp -a /var/lib/codex-router "$backup_dir/"

# 确认备份成功后，替换并启动
sudo install -m 0755 codex-router /usr/local/bin/codex-router && \
sudo systemctl start codex-router

sudo systemctl status codex-router --no-pager
sudo journalctl -u codex-router -n 50 --no-pager
```

任一步骤报错时先处理错误，再继续。只替换二进制时无需 `daemon-reload`。
更新会短暂中断请求；不要用新版 `config.example.toml` 覆盖现有 `config.toml`。
配置模板仅用于对照新增设置，手工改配置前应停止服务，保存后再启动。

备份包含账户凭据及数据库、WAL 等文件；如果配置了目录外的数据库路径，停服后还需单独备份
该数据库及其 `-wal`、`-shm` 文件（若存在）。新版可能迁移持久状态，应保留更新前的备份。

## 6. 回滚二进制

如果新版启动失败，且持久状态仍与旧版兼容：

```sh
sudo systemctl stop codex-router && \
sudo install -m 0755 /usr/local/bin/codex-router.bak /usr/local/bin/codex-router && \
sudo systemctl start codex-router
sudo systemctl status codex-router --no-pager
```

如果新版已迁移配置或数据库，单独回滚二进制可能不够，需要停服并恢复更新前配套的数据备份；
恢复会丢失备份之后的状态和用量记录。
