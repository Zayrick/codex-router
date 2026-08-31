# Codex Router

Codex Router 是一个独立运行的 Rust API 网关，把 ChatGPT Codex 能力转换或映射为 OpenAI、
Anthropic 与 Gemini 风格接口。服务使用 Tokio、Axum 和 Reqwest，配置与账户状态保存在本地
TOML 文件中，下游 Token 用量保存在 SQLite 中，React 管理界面随发行二进制一起提供。

## 当前能力

- OpenAI Models、Responses、Chat Completions 与 legacy Completions；
- Anthropic Messages 与本地 token count；
- Gemini Models、generateContent、streamGenerateContent 与 countTokens；
- `/backend-api/*` 和未注册路径的透明 HTTP/SSE/WebSocket 转发；
- Codex Responses、图片、Realtime/Live、multipart 与二进制流式代理；
- OAuth 设备授权、下游 API Key、代理账户及代理账户独立 OAuth 的管理 JSON API；
- React 管理页面与账户用量查询页；
- 按 API Key、代理账户、模型和 HTTP/WebSocket 统计实际 Codex Token 用量；
- 后台 OAuth 刷新、用量采集、reset watch、Bark 与钉钉通知；
- 原生流式正文和双向 WebSocket bridge。

`GET /` 提供账户用量查询页，提交 API Key 或 account id 后在原页显示对应调用身份的用量；管理页面只在精确的
`/<admin.path>/admin` 路径提供，附近路径不会暴露页面。发行构建会把带指纹的 React 资源直接
嵌入 Rust 二进制，部署时不需要 Node.js、pnpm 或 `frontend` 目录。完整契约见
[API 文档](docs/api.md)，运行与持久化设置见[配置文档](docs/configuration.md)。

## 运行

要求 Rust 1.97 或更高版本。第一次从源码构建还需要 Node.js 22.12 或更高版本及 pnpm 11；它们只
用于编译 React 前端，不是发行二进制的运行时依赖。

```sh
cp config.example.toml config.toml
cargo run -- --config config.toml
```

开发模式下这一个命令会同时启动 Vite 和 Rust 服务。修改 React/CSS 后浏览器通过 HMR 更新；修改
Rust 源码后会自动执行增量编译，并在编译成功后重启后端。请通过 Rust 服务访问页面：

- `http://127.0.0.1:8787/`
- `http://127.0.0.1:8787/<admin.path>/admin`

Vite 的 `127.0.0.1:5173` 端口只提供开发资源。生产运行使用发行构建：

```sh
cargo run --release -- --config config.toml
```

`cargo build --release` 同样会自动安装、检查并构建前端，然后把产物嵌入
`target/release/codex-router`。构建完成后，单独复制该二进制和配置文件即可运行。

不传 `--config` 时默认读取当前目录的 `config.toml`。配置只从该文件和命令行路径读取，不读取
业务环境变量。

启动前至少修改以下项目：

- `admin.path`：隐藏管理 API 的 URL 段；
- `admin.secret`：管理登录密钥；
- `usage_tracking.database_path`：Token 用量 SQLite 文件，默认位于配置文件旁的 `usage.sqlite3`；
- `state.api_keys`：至少一个启用的下游 API Key，或启动后通过管理 API 创建。

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

## 配置与持久状态

`config.toml` 同时保存静态设置和运行时状态：

- 主 OAuth 位于 `state.oauth`；
- 下游 Key 位于 `state.api_keys`；
- 代理账户位于 `state.auth_proxy_accounts`；
- 代理 OAuth 位于 `state.auth_proxy_oauth`；
- 订阅额度快照位于 `state.usage`。

这些字段直接保存在 `config.toml`。管理 API 和后台刷新写入状态时，会先写入权限为 `0600` 的
临时文件，再原子替换原配置。成功写入会把 TOML 规范化，注释可能丢失；请保留单独的配置模板，
不要在管理 API 写入期间同时手工编辑生产配置。手工修改静态设置后需要重启服务。

下游 Token 用量单独写入 `usage_tracking.database_path` 指定的 SQLite 数据库。相对路径以配置文件
目录为基准；管理页提供 24 小时、7 天、30 天和全部范围的趋势、模型/身份拆分及最近请求明细，
并可按具体 API Key 或下游 account id 重新计算全部用量视图。

管理会话和 OAuth 设备轮询状态使用 HMAC-SHA256 签名。改变 `admin.secret` 会立即使已有管理会话
与未完成的设备授权 state 失效。

## 部署注意

- `upstream.chatgpt_proxy` 如包含认证信息，应与 OAuth 和 API Key 一样作为密钥保护；SOCKS5
  连接上的 ChatGPT 应用流量仍使用端到端 TLS；
- `server.public_origin` 必须是客户端看到的精确 origin，并用于管理 API 同源校验；
- 管理 Cookie 始终带 `Secure`，生产和浏览器管理场景应通过 HTTPS 反向代理访问；
- 反向代理需要允许 WebSocket Upgrade，并关闭 SSE 响应缓冲；
- `config.toml` 包含明文 OAuth、API Key、Webhook 和管理密钥，不应提交到版本库、复制到日志或
  放入权限宽松的备份。
- `usage.sqlite3` 及其 `-wal`/`-shm` 文件包含用量与身份元数据，也应按私有运行数据保护和备份。

## 开发检查

依赖统一通过不带版本号的 `cargo add` 安装，由 Cargo 选择当前最新版。常用检查：

```sh
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
(cd frontend && pnpm typecheck && pnpm lint && pnpm build)
```
