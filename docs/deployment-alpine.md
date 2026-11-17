# Alpine 替换部署

本次部署目标为 `root@192.168.160.172`，使用 Alpine/OpenRC。程序由 Docker 构建为
x86_64 musl 静态二进制，在服务器上作为原生服务运行。

## 入口与服务

| 用途 | 地址 / 服务 |
| --- | --- |
| API | `http://192.168.160.172:8317/v1` |
| 局域网管理控制台 | `http://192.168.160.172:8317/admin` |
| 旧账户 ID 客户端 | `http://192.168.160.172:8318/backend-api/codex` |
| Windows 安装脚本 | `http://192.168.160.172:8320/install.ps1` |
| macOS 安装脚本 | `http://192.168.160.172:8320/install.sh` |
| API 进程 | `codex-router` |
| 注册服务 | `codex-router-bootstrap` |

管理员密钥保存在服务器 `/etc/codex-router/admin-secret`，可以通过 SSH 读取。
浏览器管理入口使用上述局域网地址，与 `server.public_origin` 一致。
nginx 在 8317 接收请求，管理路径仅允许实际连接来源为 `192.168.0.0/16`；HTTP、HTTPS 和端口属于不同来源。
后端仅监听 `127.0.0.1:18317`，Bootstrap 通过该回环地址调用管理 API。
局域网控制台启用 `session.allow_insecure_http = true`，登录凭据通过内网 HTTP 传输。

原 `cliproxyapi`、`cpa-usage-keeper`、`codex-account-proxy`、`codex-bootstrap` 已停用并移出
默认启动级别。根用户 crontab 中的 CPA 自动升级及数据库备份清理任务同时移除。
Cloudflare Tunnel、临时 Key 网关和 FRP 的 API 入口沿用原地址，并通过 8317 的 nginx 网关访问后端。
公网隧道和临时 Key 网关连接 nginx 时的来源是回环地址，因此管理路径返回 `403`；端口 80 的 `codex.ps1` 继续提供下载。

## 迁移数据

`deploy/migrate_cpa.py` 读取 CLIProxyAPI YAML、OAuth JSON 及 Bootstrap `users.json`，生成私有 TOML：

- 26 个原始 API Key 按原值迁入独立用户，5 个命名注册用户保留原名字。
- 3 个 Codex 账户保留 Access Token、Refresh Token、ID Token、账户标识和有效期。
- 账户加入 `CPA migrated pool`，启用轮询及原有会话亲和设置。
- 已采样的额度快照带入正式配置，额度耗尽的账户由 Router 隔离，巡检恢复额度后重新参与调度。

旧服务的账户 `priority` 保存在迁移报告和原始备份中；新服务按账户组策略及额度状态调度。
CPA Usage Keeper 历史数据保存在备份中的 `app.db` 及原目录，新 Router 从切换时开始记录用量。

8318 的 nginx 将 `/backend-api/codex/*` 按原路径、查询参数、正文和认证头直接转发到
`127.0.0.1:18317`。后端根据 `account_id` 凭据匹配用户并处理原生账户路由；凭据类型由管理端维护。
该端口提供 Codex 原生路径及 `/healthz`，管理控制台使用 8317 的局域网入口。

nginx 设置 `client_max_body_size 128m`，关闭请求和响应缓冲，支持 WebSocket Upgrade，读写超时
为一小时。Router 的 JSON 上限为 100 MiB，图片 multipart 为 64 MiB，Live bootstrap 为 16 MiB。

## Bootstrap

`deploy/bootstrap/server.py` 保持 `/register`、`/install.ps1`、`/install.sh` 和两个配置助手下载地址。
同一名字重复注册会取得原 Key。注册通过回环地址登录 Router 管理 API，创建用户并分配账户组；
已存在的自定义路由会保留，已禁用凭据返回注册失败。

注册记录先原子保存到 `/var/lib/codex-router-bootstrap/users.json`，再调用管理 API。
进程重启或路由写入失败后，重试使用原 Key 继续完成注册。该入口沿用可信局域网内按名字领取 Key 的流程。

注册配置位于 `/etc/codex-router/bootstrap.toml`。Windows/macOS 配置助手沿用既有发行文件，
每次启动根据实际文件生成 SHA-256 校验值。安装脚本通过模型目录检查 Key 生效情况。

## 构建与检查

在仓库根目录运行：

```sh
docker build -f deploy/Dockerfile.alpine --output type=local,dest=.deployment-work/artifact .
```

构建过程执行前端 lint、HTTP 环境下的凭据弹窗回归测试、TypeScript 检查、Vite 构建和 Rust release 编译，并检查静态链接。
迁移和注册测试在 Linux 上运行，需要 Python 3.11+ 与 PyYAML：

```sh
python3 -m unittest discover -s deploy/tests -v
cargo test --locked upstream::codex::tests --lib
cargo fmt --all -- --check
```

模型目录使用上游要求的 `client_version` 查询参数，默认值为实测可用的 `0.168.0`，
可在管理端“设置 → 上游 → Codex 客户端版本”修改，保存后即时生效。
协议 API 的上游请求头由账户认证、正文类型和 Codex 控制字段组成。
`gpt-6-sol`、`gpt-6.1-sol` 已通过实际生成测试。

## 备份与切换

`deploy/cutover_alpine.py` 用于首次迁移、已在 18317/18320 验证好的实例。网关安装后的更新通过版本目录
及 `current` 指针切换，保留 `127.0.0.1:18317` 监听和 nginx 配置。首次迁移脚本停止旧新进程，备份旧程序、配置、
OAuth、注册表、Keeper 数据库及启动脚本，校验压缩包和 SQLite，再以最新旧数据生成正式配置。
备份位于 `/var/backups/codex-router-migration/<时间>/`，仅 root 可访问，路径同时写入
`/etc/codex-router/rollback-dir`。`legacy.tar.gz` 的校验值记录在同目录 `SHA256SUMS`。
切换失败时脚本恢复旧服务和原 crontab。

测试实例的数据另存在该备份的 `preflight-router/` 与 `preflight-bootstrap/`。
正式数据目录为 `/var/lib/codex-router`，包括 `config.toml`、`usage.sqlite3` 和 `sessions.sqlite3`。
运行中的配置由 Router 原子维护；人工修改配置前先停服务。

## 一条命令回滚

在本机运行：

```sh
ssh root@192.168.160.172 /usr/local/sbin/codex-router-rollback
```

回滚会校验备份、停止新服务并保存当前数据，将最新 OAuth 和已启用的用户凭据导回旧服务，
合并新增 Bootstrap 名字记录，然后移除 8318 转发，恢复四个旧服务、开机启动和原定时任务。
存在局域网控制台网关时，回滚后的 CLIProxyAPI 使用 `127.0.0.1:18317`，Keeper 通过同一回环地址采集数据，
nginx 保持管理路径的 `192.168.0.0/16` 来源限制和现有 API 入口。
这一步同步刷新后的 OAuth，便于长时间运行新服务后仍能切回。

需要恢复迁移时完整旧快照时使用：

```sh
ssh root@192.168.160.172 '/usr/local/sbin/codex-router-rollback --snapshot'
```

`--snapshot` 恢复备份时的数据；之后的 Key 和 Token 变化保存在
`/var/backups/codex-router-before-rollback/<时间>/`，可据此人工恢复。

## 日常检查

```sh
rc-service codex-router status
rc-service codex-router-bootstrap status
nginx -t
curl -i http://127.0.0.1:8317/healthz
tail -n 50 /var/log/codex-router/service.log
```

健康检查正常返回 `204`。日志通过 `/etc/logrotate.d/codex-router` 轮转；完整备份保留供回滚使用。
