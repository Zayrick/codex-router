# 配置

Codex Router 从 `config.toml` 读取运行设置和持久状态。可复制 `config.example.toml` 作为起点；
`server`、`admin`、`upstream`、`usage_tracking`、`notifications`、`public_account`、`frontend` 与 `session`
是运行设置，`state` 由管理 API
和后台任务维护。配置文件包含凭据，应使用严格的文件权限并避免提交到版本库。

## 静态设置

`server.bind` 必须是 IP socket address，例如 `127.0.0.1:8787` 或 `0.0.0.0:8787`。
`server.public_origin` 是客户端实际访问的精确 HTTP/HTTPS origin，不允许路径、Query 或尾部斜杠。
它用于解析入站请求路径、校验控制台写请求的 `Origin`，并决定会话 Cookie 是否带 `Secure`。

`server.trusted_proxies` 列出可信反向代理的 IP 或 CIDR，默认为空。只有 TCP 对端属于其中时，
登录限流才会从 `X-Forwarded-For` 读取真实客户端地址。同机 Nginx/Caddy 部署通常设置为
`["127.0.0.1", "::1"]`；代理必须覆盖或追加（而不是透传）客户端发来的该 header。

## Web 控制台

`admin.secret` 是管理员在 `/admin` 的登录密钥。控制台路径是公开的，请使用足够长的随机值
（建议至少 32 个随机字节）。已弃用的 `admin.path` 可以删除；保留时只用于把旧地址跳转到 `/admin`。

`frontend.dir` 指向前端构建产物目录（包含 `index.html` 和 `assets/`），默认 `frontend/dist`，
相对路径以配置文件目录为基准。`index.html` 每次请求时读取，因此单独更新前端无需重启服务。

`session.database_path` 是会话数据库，默认 `sessions.sqlite3`。`session.allow_insecure_http` 默认
关闭；`public_origin` 是非回环地址的 HTTP 时，管理员无法登录，避免管理 Cookie 在网络上明文传输。
会话时长、续期与失效规则见 [API 文档](api.md#11-web-控制台与登录会话)。

`state.session_binding_key` 在首次启动时自动生成，用于把会话绑定到登录时的凭据。不要通过日志或
备份泄露；删除或更换后，所有控制台用户需要重新登录。

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

`upstream.codex_client_version` 是模型目录请求的默认客户端版本，默认 `0.168.0`，格式为数字
`x.y.z`。可在管理端“设置 → 上游”修改，保存后对后续请求即时生效。模型查询优先使用请求中的
`client_version` 参数，其次使用客户端提交的 `version` 值，最后使用该配置。

通知字段都是可选的。管理端“设置”页可在运行时配置重置预测、额度重置和用量过高通知，选择全部
或指定 Codex 账户，并分别启用 Bark、钉钉通道。账户型通知会包含账户昵称。钉钉 Webhook 与签名
secret 必须同时配置；只提供其中一个会导致启动失败。`upstream.codex_resets_url` 也可在设置页更新，
仅接受 HTTPS 地址。

`public_account.show_quota` 默认开启。开启后，控制台用户视图会根据下游凭据的路由目标展示额度；若
目标是账户组，则展示组内全部 Codex 账户的昵称与额度时间轴。关闭后用户端不返回额度数据，但
Token 用量查询仍然可用。

## Token 用量数据库

实际下游 Codex 用量保存在独立的 SQLite 数据库中：

```toml
[usage_tracking]
database_path = "usage.sqlite3"
fast_cost_multiplier = 2.0
```

`fast_cost_multiplier` 是 Fast 模式（上游请求 `service_tier = "priority"`）的计费倍率，省略时为 2，
可在管理页“设置”中修改，取值范围为大于 0 且不超过 100。它在模型价格基础上叠加，仅影响用量面板的
成本展示，修改后对历史事件同样生效。

相对路径以配置文件所在目录为基准；绝对路径原样使用。服务启动时会自动创建父目录和数据库，启用
WAL 模式，并在 Unix 上将数据库、`-wal` 和 `-shm` 文件权限设置为 `0600`。修改路径后需要重启。

数据库按终止响应事件记录用户 ID/名称、所用凭据的类型/ID/备注、模型、HTTP/WebSocket 传输、端点、状态以及
输入、缓存命中、缓存创建、输出、推理和总 Token。缓存与推理字段是输入/输出的子集，不应再次
加到总量中。数据库不保存 API Key、OAuth 凭据、请求正文或响应正文，当前也不会自动清理历史
事件；备份时应将数据库及其 WAL sidecar 视为一组私有运行数据。

统计、请求日志和 CSV 按记录中的稳定 ID 关联当前用户、凭据、Codex 账户及账户组名称，保存改名后即可显示。
已删除对象使用请求记录中的名称快照；历史请求的数量和 Token 用量保持原始记录。

## 明文状态格式

账户目录、账户组与用户路由统一保存在 `state.account_routing`，用户及其凭据保存在 `state.users`。
账户组在账户组页维护；创建或编辑用户时可以直接选择单个账户或账户组。以下示例展示字段形状，
实际运行建议交给管理 API 写入：

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
userId = "00000000-0000-4000-8000-000000000003"
targetType = "group" # 或 "account"
targetId = "00000000-0000-4000-8000-000000000002"

[[state.users]]
id = "00000000-0000-4000-8000-000000000003"
name = "张三"
enabled = true

[[state.users.credentials]]
id = "00000000-0000-4000-8000-000000000004"
type = "api_key" # 或 "account_id"
value = "sk-..."
name = "laptop" # 可选备注
enabled = true
```

一个用户可以有多个 API Key 和账户 ID，任一凭据的请求都按该用户路由、计入该用户的用量，也都能
登录到该用户的看板。凭据值在全部用户之间唯一。旧版的 `state.api_keys`、`state.auth_proxy_accounts`
和不带 `userId` 的路由不再读取，启动时会被丢弃，需要在“用户”页重新录入。

`strategy` 可设为 `round-robin`、`weighted-round-robin` 或 `fallback`。加权轮询的权重无需手工配置，
由每账户额度快照中各 Codex 窗口的平均“剩余百分比 ÷ 距离刷新剩余分钟”乘以套餐倍率计算
（`pro` 为 20，`prolite` 及 `*_prolite` 为 5，其余为 1）。`round-robin` 和
`weighted-round-robin` 可配置 `sessionAffinity` 与 TTL；`fallback` 按用户固定账户。

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

手工录入的 Codex 账户、账户组、用户和凭据必须使用合法 UUID，名称在各自集合内唯一。
管理 API 会校验引用并把更新原子写回配置文件。旧版 OAuth 与额度字段会在启动时自动迁移。

## 配置文件写入语义

管理 API 和后台 OAuth/订阅额度任务共享一个进程内写锁。每次更新先生成完整 TOML 临时文件并同步到
磁盘，再原子替换正式文件；持久化失败时不会提交内存变更。Unix 上新文件权限为 `0600`。

当前不监视外部文件变化。手工编辑 `config.toml` 后需要重启进程；运行时不要同时由其他程序改写
该文件。自动写回会规范化 TOML，原文件注释不保证保留。
