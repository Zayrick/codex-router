# 配置

Codex Router 从 `config.toml` 读取运行设置和持久状态。可复制 `config.example.toml` 作为起点；
`server`、`admin`、`upstream`、`usage_tracking`、`notifications` 与 `public_account` 是运行设置，`state` 由管理 API
和后台任务维护。配置文件包含凭据，应使用严格的文件权限并避免提交到版本库。

## 静态设置

`server.bind` 必须是 IP socket address，例如 `127.0.0.1:8787` 或 `0.0.0.0:8787`。
`server.public_origin` 是客户端实际访问的精确 HTTP/HTTPS origin，不允许路径、Query 或尾部斜杠。
它用于解析入站请求路径和校验管理写请求的 `Origin`。

ChatGPT 请求发往 `https://chatgpt.com`。默认直接连接；需要代理时可配置
`upstream.chatgpt_proxy`：

```toml
[upstream]
chatgpt_proxy = "socks5h://127.0.0.1:1080"
```

该字段只接受不带路径、Query 或 Fragment 的 `socks5://` 或 `socks5h://` URL，未写端口时默认
使用 `1080`。`socks5` 在本机解析目标域名，`socks5h` 由代理解析；HTTP、SSE 和 WebSocket
上游连接使用同一配置。代理需要用户名和密码时可写成
`socks5h://user:password@127.0.0.1:1080`，特殊字符必须进行 URL percent-encoding。代理凭据会以
明文保存在配置文件中。

通知字段都是可选的。管理端“设置”页可在运行时配置重置预测、额度重置和用量过高通知，选择全部
或指定 Codex 账户，并分别启用 Bark、钉钉通道。账户型通知会包含账户昵称。钉钉 Webhook 与签名
secret 必须同时配置；只提供其中一个会导致启动失败。`upstream.codex_resets_url` 也可在设置页更新，
仅接受 HTTPS 地址。

`public_account.show_quota` 默认开启。开启后，根目录用户端会根据下游凭据的路由目标展示额度；若
目标是账户组，则展示组内全部 Codex 账户的昵称与额度时间轴。关闭后用户端不返回额度数据，但
Token 用量查询仍然可用。

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

账户目录、账户组与调用身份分配统一保存在 `state.account_routing`。账户组在 Codex 账户页维护；
创建或编辑 API Key、下游账户时可以直接选择单个账户或账户组。以下示例展示字段形状，实际运行
建议交给管理 API 写入：

```toml
[[state.account_routing.accounts]]
id = "00000000-0000-4000-8000-000000000001"
name = "Personal"
enabled = true

[[state.account_routing.groups]]
id = "00000000-0000-4000-8000-000000000002"
name = "Production Pool"
accountIds = ["00000000-0000-4000-8000-000000000001"]
strategy = "round-robin"
sessionAffinity = true
sessionAffinityTtl = "1h" # 或 "unlimited"

[[state.account_routing.routes]]
consumerType = "api_key" # 或 "auth_proxy"
consumerId = "00000000-0000-4000-8000-000000000003"
targetType = "group" # 或 "account"
targetId = "00000000-0000-4000-8000-000000000002"
```

`strategy` 可设为 `round-robin`、`weighted-round-robin` 或 `fallback`。加权轮询的权重无需手工配置，
由每账户额度快照中各 Codex 窗口的平均“剩余百分比 ÷ 距离刷新剩余分钟”计算。`round-robin` 和
`weighted-round-robin` 可配置 `sessionAffinity` 与 TTL；`fallback` 按调用身份固定账户。

`server.maintenance_interval_seconds` 控制 OAuth 与额度巡检间隔，默认值为 300 秒。

每个 Codex 账户的 OAuth 使用账户 UUID 作为 TOML 表 key；字段使用 camelCase，与管理 API JSON
一致：

```toml
[state.codex_account_oauth."00000000-0000-4000-8000-000000000001"]
version = 1
accessToken = "..."
refreshToken = "..."
idToken = "..."
accountId = "account-id"
email = "user@example.com"
expiresAt = 1800003600000
updatedAt = "2027-01-15T09:00:00.000Z"
```

手工录入的 Codex 账户、账户组、API Key 和下游账户必须使用合法 UUID，名称在各自集合内唯一。
管理 API 会校验引用并把更新原子写回配置文件。旧版 OAuth 与额度字段会在启动时自动迁移。

## 配置文件写入语义

管理 API 和后台 OAuth/订阅额度任务共享一个进程内写锁。每次更新先生成完整 TOML 临时文件并同步到
磁盘，再原子替换正式文件；持久化失败时不会提交内存变更。Unix 上新文件权限为 `0600`。

当前不监视外部文件变化。手工编辑 `config.toml` 后需要重启进程；运行时不要同时由其他程序改写
该文件。自动写回会规范化 TOML，原文件注释不保证保留。
