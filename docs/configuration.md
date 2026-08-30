# 配置与迁移

## Worker 配置映射

| 原 Worker 项目 | 当前配置 |
| --- | --- |
| `CORS_ORIGIN` | `server.cors_origin` |
| `ADMIN_PATH` | `admin.path` |
| `ADMIN_SECRET` | `admin.secret` |
| `CHATGPT_RELAY_URL` | `upstream.chatgpt_relay_url` |
| `BARK_PUSH_URL` | `notifications.bark_push_url` |
| `DINGTALK_WEBHOOK_URL` | `notifications.dingtalk_webhook_url` |
| `DINGTALK_SECRET` | `notifications.dingtalk_secret` |
| `DATA_ENCRYPTION_KEY` | 已删除 |
| `AUTH_KV` 的主 OAuth | `state.oauth`，明文 |
| `AUTH_KV` 的 API Key | `state.api_keys`，明文 |
| `AUTH_KV` 的代理账户 | `state.auth_proxy_accounts`，明文 |
| `AUTH_KV` 的代理 OAuth | `state.auth_proxy_oauth`，明文 |
| `AUTH_KV` 的用量快照 | `state.usage`，明文 |
| Cron Trigger | 进程内定时任务，周期为 `server.maintenance_interval_seconds` |
| `ASSETS` binding | 已删除；React 资源在发行构建时嵌入 Rust 二进制 |

旧 KV 中的数据是 AES-GCM 密文，当前服务不会读取 KV 或旧 envelope。迁移时可通过当前管理 API
重新完成 OAuth 设备授权并重新创建 API Key/代理账户，也可以在可信离线环境中解密旧数据后，按
`config.example.toml` 的结构手工写入 `state`。不要把旧 `DATA_ENCRYPTION_KEY` 放入新配置；当前
程序没有该字段，也不会使用它。

## 静态设置

`server.bind` 必须是 IP socket address，例如 `127.0.0.1:8787` 或 `0.0.0.0:8787`。
`server.public_origin` 是客户端实际访问的精确 HTTP/HTTPS origin，不允许路径、Query 或尾部斜杠。
它用于构造 relay 请求 URL 和校验管理写请求的 `Origin`。

`upstream.chatgpt_relay_url` 必须是与 `server.public_origin` 不同的精确 HTTPS origin。relay 属于
高信任边界，会看到 OAuth、账户 ID、提示、工具参数、上传内容和模型输出。

通知字段都是可选的。钉钉 Webhook 与签名 secret 必须同时配置；只提供其中一个会导致启动失败。

## Token 用量数据库

实际下游 Codex 用量保存在独立的 SQLite 数据库中：

```toml
[usage_tracking]
database_path = "usage.sqlite3"
```

相对路径以配置文件所在目录为基准；绝对路径原样使用。服务启动时会自动创建父目录和数据库，启用
WAL 模式，并在 Unix 上将数据库、`-wal` 和 `-shm` 文件权限设置为 `0600`。修改路径后需要重启。

数据库按终止响应事件记录身份类型、身份 ID/名称、模型、HTTP/WebSocket 传输、端点、状态以及
输入、缓存命中、缓存创建、输出、推理和总 Token。缓存与推理字段是输入/输出的子集，不应再次
加到总量中。数据库不保存 API Key、OAuth 凭据、请求正文或响应正文，当前也不会自动清理历史
事件；备份时应将数据库及其 WAL sidecar 视为一组私有运行数据。

## 明文状态格式

OAuth 字段使用 camelCase，与管理 API 的 JSON 结构一致：

```toml
[state.oauth]
version = 1
accessToken = "..."
refreshToken = "..."
idToken = "..."
accountId = "..."
email = "user@example.com"
expiresAt = 1800003600000
updatedAt = "2027-01-15T09:00:00.000Z"
```

代理账户 OAuth 使用账户 UUID 作为 TOML 表 key：

```toml
[state.auth_proxy_oauth."00000000-0000-4000-8000-000000000001"]
version = 1
accessToken = "..."
refreshToken = "..."
accountId = "account-id"
expiresAt = 1800003600000
updatedAt = "2027-01-15T09:00:00.000Z"
```

手工录入的 API Key 和代理账户必须使用合法 UUID。通过管理 API 创建时，服务会自动生成 UUID、
校验唯一性并把更新原子写回配置文件。

## 配置文件写入语义

管理 API 和后台 OAuth/订阅额度任务共享一个进程内写锁。每次更新先生成完整 TOML 临时文件并同步到
磁盘，再原子替换正式文件；持久化失败时不会提交内存变更。Unix 上新文件权限为 `0600`。

当前不监视外部文件变化。手工编辑 `config.toml` 后需要重启进程；运行时不要同时由其他程序改写
该文件。自动写回会规范化 TOML，原文件注释不保证保留。
