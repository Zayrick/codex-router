# Codex Router

Codex Router 是一个独立运行的 Rust API 网关，把 ChatGPT Codex 能力转换或映射为 OpenAI、
Anthropic 与 Gemini 风格接口。服务使用 Tokio、Axum 和 Reqwest，配置与账户状态保存在本地
TOML 文件中，下游 Token 用量保存在 SQLite 中，React 控制台作为独立的静态文件目录随发行包提供。

## 当前能力

- OpenAI Models、Responses、Chat Completions、legacy Completions 与 Images 生成/编辑；
- Anthropic Messages 与本地 token count；
- Gemini Models、generateContent、streamGenerateContent 与 countTokens；
- `/backend-api/*` 和未注册路径的透明 HTTP/SSE/WebSocket 转发；
- 标准图片编辑 multipart 转 Codex JSON、图像模型目录补充、Base64/Data URL 与图像 SSE；
- Codex Responses、Realtime/Live 和其他原生 multipart、二进制流式代理；
- 统一 Codex 账户池、设备授权、JSON / 手动凭据导入、账户组与用户管理 API；一个用户可持有多个 API Key 和账户 ID；
- 统一的 `/admin` 控制台：管理员与用户同一入口登录（用户可用自己的任一凭据登录同一看板），登录状态保存在服务端会话中并自动续期；
  用户视图的账户组可展示组内全部额度时间轴；
- 按用户、单个凭据、Codex 账户/组、模型和 HTTP/WebSocket 统计实际 Token 用量；
- 后台 OAuth 刷新、用量采集，以及可按事件和账户配置的 reset watch、Bark 与钉钉通知；
- 原生流式正文和双向 WebSocket bridge。

控制台位于 `/admin`（`/` 会跳转过去）。输入 `admin.secret` 进入管理视图，输入用户的任一 API Key
或账户 ID 进入该用户的用量视图。前端不再嵌入二进制：服务从 `frontend.dir` 目录读取
`index.html` 并为每次请求注入 CSP nonce，`/admin/assets/*` 直接读取该目录下的带指纹资源。
服务器只需要构建好的静态文件，不需要 Node.js 或 pnpm。完整契约见
[API 文档](docs/api.md)，运行与持久化设置见[配置文档](docs/configuration.md)。

## 运行

要求 Rust 1.97 或更高版本。开发和构建前端还需要 Node.js 22.12 或更高版本及 pnpm 11；它们不是
服务运行时的依赖。

```sh
cp config.example.toml config.toml
cargo run -- --config config.toml
```

开发模式下这一个命令会同时启动 Vite 和 Rust 服务。修改 React/CSS 后浏览器通过 HMR 更新；修改
Rust 源码后会自动执行增量编译，并在编译成功后重启后端。请通过 Rust 服务访问页面：

- `http://127.0.0.1:8787/admin`

Vite 的 `127.0.0.1:5173` 端口只提供开发资源。生产运行先单独构建前端，再运行发行二进制：

```sh
(cd frontend && pnpm install --frozen-lockfile && pnpm build)
cargo run --release -- --config config.toml
```

`cargo build` 不再构建前端。`frontend.dir` 默认是配置文件旁的 `frontend/dist`；部署时把
`frontend/dist` 的内容复制到服务器任意目录，并在配置中指向它。更新前端只需替换该目录，无需重启。

不传 `--config` 时默认读取当前目录的 `config.toml`。配置只从该文件和命令行路径读取，不读取
业务环境变量。

启动前至少修改以下项目：

- `admin.secret`：管理员登录密钥（控制台路径公开，请使用足够长的随机值）；
- `frontend.dir`：前端构建产物目录；
- `server.trusted_proxies`：同机反向代理部署时设为 `["127.0.0.1", "::1"]`，让登录限流识别真实客户端；
- `usage_tracking.database_path`：Token 用量 SQLite 文件，默认位于配置文件旁的 `usage.sqlite3`；
- 登录或导入至少一个 Codex 账户，按需创建账户组，并在创建或编辑用户时选择账户或账户组。

ChatGPT 请求发往 `https://chatgpt.com`。默认直接连接；如需通过 SOCKS5 出站，在 `[upstream]`
中配置 `chatgpt_proxy = "socks5h://127.0.0.1:1080"`。HTTP、SSE 和 WebSocket 会使用同一代理；完整
格式与 DNS 解析差异见[配置文档](docs/configuration.md)。

验证本地接口：

```sh
curl http://127.0.0.1:8787/v1/messages/count_tokens \
  -H 'Authorization: Bearer sk-change-me-123!' \
  -H 'Content-Type: application/json' \
  -d '{"model":"gpt-5","messages":[{"role":"user","content":"hello"}]}'
```

## 统一账户调度

设备登录和凭据导入会向 Codex 账户池添加账户。用户可路由到单个账户或账户组，其全部凭据共用这条路由；
未分配的用户用 API Key 调用时返回 `404`，用账户 ID 转发时按来访凭据透明转发。

账户组支持三种策略：

- `round-robin`：按顺序平均轮询；
- `weighted-round-robin`：根据账户剩余额度和刷新时间平滑加权；
- `fallback`：为每个用户固定账户，并在账户不可用时切换。

前两种策略支持 session affinity，TTL 接受 `30m`、`1h`、`7d` 等时长或 `unlimited`。禁用账户会
暂停调度并释放直连路由；上游返回 `429` 时，调度器会隔离该账户，直到额度巡检确认恢复。

## 凭据导入

管理页面的“Codex 账户”中点击“导入凭据”，可上传 / 粘贴单个账户的 `auth.json`，或切换到
“手动填写”。至少需要 `refresh_token`；`access_token`、`id_token`、`account_id`、邮箱和到期时间
可选。系统会从 Token 提取账户信息；只提供 Refresh Token，或 Access Token 缺失、过期、有效期
未知时，会先向现有 OAuth 上游刷新。无法识别 Account ID 时需要补填。导入成功后无需再进行设备
登录，后台会按现有机制自动续期。具体 JSON 格式见 [凭据导入 API](docs/api.md#凭据导入)。

## 配置与持久状态

`config.toml` 同时保存静态设置和运行时状态：

- 统一账户目录、账户组和路由位于 `state.account_routing`；
- 每个 Codex 账户的 OAuth 位于 `state.codex_account_oauth`；
- 用户及其 API Key、账户 ID 位于 `state.users`；
- 每账户订阅额度快照位于 `state.account_usage`。

启动时会自动迁移旧版 OAuth 与额度状态。

这些字段直接保存在 `config.toml`。管理 API 和后台刷新写入状态时，会先写入权限为 `0600` 的
临时文件，再原子替换原配置。成功写入会把 TOML 规范化，注释可能丢失；请保留单独的配置模板，
不要在管理 API 写入期间同时手工编辑生产配置。手工修改静态设置后需要重启服务。

下游 Token 用量单独写入 `usage_tracking.database_path` 指定的 SQLite 数据库。相对路径以配置文件
目录为基准；管理页提供 24 小时、7 天、30 天和全部范围的趋势、模型/用户拆分及最近请求明细，
并可按用户、单个凭据、Codex 账户或账户组重新计算全部用量视图。用户端时间筛选
提供 24 小时、7 天、30 天和全部范围；用户端额度是否显示、通知和模型价格统一在管理端“设置”页维护。

控制台会话保存在 `session.database_path`（默认 `sessions.sqlite3`）。管理员会话空闲 24 小时或满 7 天
过期，用户会话空闲 30 天或满 90 天过期，使用期间自动续期。修改 `admin.secret`，或停用、删除用户，
以及修改、停用、删除用户已启用的凭据，都会让相关会话立即失效。OAuth 设备轮询 state 仍使用
`admin.secret` 进行 HMAC-SHA256 签名。

## 部署注意

在 GitHub Actions 页面手动运行对应工作流，下载 amd64（x86_64）构建产物：

| 运行环境 | 工作流 | 产物 |
| --- | --- | --- |
| Ubuntu 22.04 或更高版本 | `Build and Deploy` | `codex-router-linux-amd64.tar.gz` |
| Alpine Linux（musl） | `Build Alpine` | `codex-router-alpine-amd64.tar.gz` |

`Build and Deploy` 的 `deploy` 开关控制 Ubuntu 服务器部署。`Build Alpine` 在 Alpine 3.22 容器中
直接编译静态链接二进制并运行版本检查，然后上传产物供下载。Alpine 运行环境需安装 `ca-certificates`
用于 HTTPS 证书校验。

Ubuntu 可按 [Ubuntu 部署与更新](docs/deployment-ubuntu.md) 安装、创建 systemd 服务、
设置开机启动，以及更新和回滚版本。

- `upstream.chatgpt_proxy` 如包含认证信息，应与 OAuth 和 API Key 一样作为密钥保护；SOCKS5
  连接上的 ChatGPT 应用流量仍使用端到端 TLS；
- `server.public_origin` 必须是客户端看到的精确 origin，并用于控制台同源校验；
- 控制台应通过 HTTPS 访问；`public_origin` 为非回环 HTTP 时默认禁止管理员登录；
- 反向代理需要允许 WebSocket Upgrade，并关闭 SSE 响应缓冲；
- `config.toml` 包含明文 OAuth、API Key、Webhook 和管理密钥，不应提交到版本库、复制到日志或
  放入权限宽松的备份。
- `usage.sqlite3`、`sessions.sqlite3` 及其 `-wal`/`-shm` 文件包含用量、身份元数据与会话，也应按
  私有运行数据保护和备份。

## 开发检查

依赖统一通过不带版本号的 `cargo add` 安装，由 Cargo 选择当前最新版。常用检查：

```sh
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
(cd frontend && pnpm typecheck && pnpm lint && pnpm build)
```
