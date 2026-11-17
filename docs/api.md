# API 与协议兼容性

## 1. 兼容范围

Codex Router 通过协议转换、Codex 原生映射和透明传输提供多种客户端接口。本文描述的是当前
仓库代码与测试所保证的行为；具体模型、额度和私有 Codex action 是否可用，仍取决于 OpenAI
账户与 ChatGPT 上游服务。

兼容级别定义如下：

| 级别 | 含义 |
| --- | --- |
| 协议转换 | 服务解析请求，转换为 Codex Responses，并生成目标协议的响应 |
| 原生映射 | 服务将公开路径映射到 Codex 路径，并执行有限的请求规范化 |
| 透明传输 | 服务负责鉴权、header 策略和流式传输，不承诺上游 action 可用 |
| 本地处理 | 服务在本地完成计算，不访问协议供应商的对应服务 |

本项目不声明完整的 OpenAI、Anthropic 或 Gemini API 兼容性。未在本文列出的供应商路径不属于
公开契约。

## 2. 鉴权与通用行为

明确注册的公开协议 API 要求一个已启用的下游 API Key；健康检查、已知 API 的预检请求、管理
入口、`/backend-api` 路径族和其他未注册路径不使用该鉴权。公开 API 支持以下 header，选择
优先级固定：

1. `Authorization: Bearer <key>`；
2. `X-Api-Key: <key>`；
3. `X-Goog-Api-Key: <key>`。

同时提供多个 header 时，服务只验证优先级最高的值，不会在验证失败后回退。缺失、错误、
已停用或尚未分配账户路由的 Key 返回空正文 `404`。

已知公开路径的 `OPTIONS` 请求无需鉴权，返回 `204` 并应用 `server.cors_origin`。管理路由不启用
CORS。

### 健康检查

| 方法与路径 | 鉴权 | 行为 |
| --- | --- | --- |
| `GET /healthz` | 无 | 统一账户池中至少一个已启用账户的 OAuth 可读取且未过期时返回空正文 `204`；其他情况返回空正文 `404` |

健康检查不验证 ChatGPT 上游可达性，也不发起上游请求。

## 3. OpenAI 与 Codex 路由

| 方法与路径 | 级别 | 行为 |
| --- | --- | --- |
| `GET /v1/models` | 协议转换 | 读取 Codex 模型目录并返回 OpenAI model list |
| `POST /v1/chat/completions` | 协议转换 | Chat 请求转换为 Responses；返回 Chat JSON 或 SSE |
| `POST /v1/completions` | 协议转换 | legacy prompt 转换为 Responses；返回 `text_completion` JSON 或 SSE |
| `POST /v1/responses` | 原生映射 | 映射到 `/backend-api/codex/responses` 并应用 Responses 创建策略 |
| `POST /v1/responses/compact` | 原生映射 | 映射到 Codex compact，仅应用 input 角色规范化 |
| `GET /v1/responses` + WebSocket Upgrade | 原生映射 | 建立 Responses 双向 WebSocket bridge |
| `/v1/responses/*` 其他子路径 | 透明传输 | 映射到同名 Codex 子路径；上游决定是否支持 |
| `/v1/images[/…]` | 透明传输 | 映射到 `/backend-api/codex/images[/…]`，支持流式 JSON、multipart 和二进制正文 |
| `/v1/alpha/search` | 透明传输 | 映射到 `/backend-api/codex/alpha/search` |
| `POST /v1/live` | 原生映射 | 映射到 Codex Realtime call bootstrap，并补充缺失的默认查询参数 |
| `POST /v1/realtime/calls` | 原生映射 | 与 `/v1/live` 使用相同的 bootstrap 逻辑 |
| `GET /v1/live/{call_id}` + WebSocket Upgrade | 透明传输 | 连接 `api.openai.com` 的 Live sideband |
| `GET /v1/realtime?call_id=…` + WebSocket Upgrade | 透明传输 | 校验 `call_id` 后连接 Realtime sideband |
| `GET /v1/realtime/calls/{call_id}` + WebSocket Upgrade | 透明传输 | 连接对应 Realtime sideband |
一般透明代理路径拒绝 `CONNECT`；`OPTIONS` 由预检逻辑处理。除表中明确限制方法的路径外，
路由层允许其他 HTTP 方法，上游能力仍由目标 action 决定。

### Backend API 与透明转发

路由按请求路径和方法选择，不按入站 Host 分流。`/backend-api` 路径族直接以原方法、路径、
Query、流式正文和端到端 header 转发到固定的 `https://chatgpt.com` 上游，并返回 HTTP、SSE 或
WebSocket 响应。配置 `upstream.chatgpt_proxy` 时这些连接通过 SOCKS5 建立。该路径族不执行下游
API Key 鉴权或协议转换。

仅路径以 `/backend-api/codex/` 开头，且请求中的 `ChatGPT-Account-ID` 精确匹配一条已启用下游
账户记录时，服务查询该调用身份的账户路由。分配到单个账户或账户组时，会使用调度得到的有效
Codex OAuth 替换原请求中已有的 `Authorization` 和 `ChatGPT-Account-ID`；尚未分配、记录已停用
或未匹配时按原认证信息透明转发。已配置路由但目标没有可用账户时返回本地错误。
其他 `/backend-api` 路径（包括不带尾部斜杠的 `/backend-api/codex`）保留原始凭据，不查询账户路由。

账户组的 `strategy` 支持以下值：

- `round-robin`：按稳定账户顺序平均轮询；
- `weighted-round-robin`：按 Codex 额度窗口的平均
  `remainingPercent / 距离 resetAt 的剩余分钟` 平滑加权，未知额度按组内平均权重处理；
- `fallback`：按 API Key 或下游 account id 固定账户，并在该账户不可用时切换。

`round-robin` 和 `weighted-round-robin` 支持 `sessionAffinity` 与 `sessionAffinityTtl`。TTL 接受
`humantime` 时长（如 `30m`、`1h`、`7d`）或 `unlimited`。启用亲和后依次识别
`x-claude-code-session-id`、`session-id`、`session_id`、`x-session-id`、`x-session-affinity` 和
`x-client-request-id`；无法取得会话 ID 时使用所选策略。

已路由请求返回 `429` 时，调度器会隔离对应账户；额度巡检确认恢复后重新启用调度。巡检间隔由
`server.maintenance_interval_seconds` 控制，默认 300 秒。

未注册的其他 HTTP 路径同样透明转发到 ChatGPT 上游，并保留 `Authorization`、
`ChatGPT-Account-ID`、Cookie（控制台会话 Cookie 除外）和其他端到端 header。健康检查、
`/admin` 控制台和已注册协议 API 按各自的本地路由处理。

当 `/v1/models` 包含 `client_version` 查询参数时，服务保留 Codex CLI 模型目录格式，而
不转换为 OpenAI model list。

## 4. Anthropic 路由

| 方法与路径 | 级别 | 行为 |
| --- | --- | --- |
| `POST /v1/messages` | 协议转换 | Anthropic Messages 转换为 Codex Responses；返回 Message JSON 或命名 SSE 事件 |
| `POST /v1/messages/count_tokens` | 本地处理 | 对转换后的 Codex input 使用本地 tokenizer 估算 `input_tokens` |

Messages 支持 system、文本、图片、文档、thinking、客户端工具、工具结果、Web Search、
tool choice、usage 和 stop reason 等主要结构。请求必须包含 Anthropic 形式的 `max_tokens`，
该值转发为 Codex 的 `max_output_tokens`（上游最小值为 16，更小的值按 16 发送）。
`temperature`、`top_p`、`top_k`、`stop_sequences`、metadata 和 `cache_control` 不提供
等价的上游语义。

错误响应使用 Anthropic error envelope；流式错误使用对应的 SSE error 事件。

## 5. Gemini 路由

| 方法与路径 | 级别 | 行为 |
| --- | --- | --- |
| `GET /v1beta/models` | 协议转换 | 返回 Gemini Model 列表 |
| `GET /v1beta/models/{model}` | 协议转换 | 返回单个 Gemini Model 资源 |
| `POST /v1beta/models/{model}:generateContent` | 协议转换 | Gemini Content 转换为 Codex Responses，并返回 candidates |
| `POST /v1beta/models/{model}:streamGenerateContent` | 协议转换 | 将 Codex SSE 转换为 Gemini SSE 数据事件 |
| `POST /v1beta/models/{model}:countTokens` | 本地处理 | 估算顶层 `contents` 或嵌套 `generateContentRequest` 的 token 数 |

转换覆盖 system instruction、Content/Part、内联或 URI 媒体、function call/result、工具声明、
tool config、thinking 和 usage metadata。`generationConfig` 中 thinking level/budget 与
`maxOutputTokens`（转为 `max_output_tokens`，最小 16）具有对应转换；采样参数、候选数、
停止序列以及输出 MIME/schema 不会转发为 Codex 语义。

错误响应使用 Google 风格 error envelope。

## 6. Responses 请求策略

`POST /v1/responses` 与 WebSocket `response.create` 应用以下共同规则：

- 字符串形式的顶层 `input` 包装为单个 `user` / `input_text` 消息；
- 数组 `input` 中消息项的 `role: "system"` 改为 `role: "developer"`；
- `store` 固定为 `false`；
- 删除 `max_completion_tokens`、`max_output_tokens`、`maxOutputTokens`、`max_tokens`、
  `context_management`、`temperature`、`top_p`、`truncation`、`user` 和
  `prompt_cache_options`；
- `service_tier` 仅在值严格等于 `priority` 时保留；
- 其他未知字段保持不变。

普通 HTTP `POST /v1/responses` 还会删除 `previous_response_id`、`generate`、
`prompt_cache_retention`、`safety_identifier` 和 `stream_options`。WebSocket
`response.create` 仅额外删除其中的 `prompt_cache_retention` 与 `safety_identifier`，保留
`previous_response_id`、`generate` 和 `stream_options` 的会话语义。

Chat Completions、旧版 Completions、Anthropic Messages 与 Gemini Content 转换后发往 Codex
Responses 的请求使用单独的策略，只移除上游明确拒绝的值：

- 删除 `background`、`generate`、`max_completion_tokens`、`max_tokens`、`maxOutputTokens`、
  `metadata`、`previous_response_id`、`prompt_cache_options`、`prompt_cache_retention`、
  `safety_identifier`、`temperature`、`top_p`、`truncation` 和 `user`；
- `service_tier` 仅在值严格等于 `priority` 时保留；
- `store` 固定为 `false`，`stream` 固定为 `true`（上游要求两者显式出现）；
- `stream_options` 只删除 `include_usage`，删空后移除整个对象；
- `include` 删除 `message.output_text.logprobs`，并补齐 `reasoning.encrypted_content`；
- 删除 `type: "configuration_update"` 输入项和内容块上的 `prompt_cache_breakpoint`；
- `max_output_tokens`、`context_management`、`prompt_cache_key` 和 `instructions` 保持不变。

Chat Completions 的 `max_completion_tokens`（优先）或 `max_tokens`、旧版 Completions 的
`max_tokens` 均转为 `max_output_tokens`，小于 16 时按 16 发送；Chat 的 `metadata` 不再转发。

compact 只应用数组 input 的角色规范化，不应用 Responses 创建参数策略。WebSocket
`response.append` 同样只规范化 input 角色；其他文本帧、所有二进制帧和全部上游帧保持原样。

正文不需要变更时保留原始编码；发生变更时重新编码为 JSON，并更新相关内容 header。

## 7. 其他协议差异

### Chat Completions

Chat 请求在内部始终通过流式 Codex Responses 执行，再根据下游 `stream` 选择聚合 JSON 或
转换为 Chat SSE。Codex 不接受的生成参数不会被伪造为等价能力。

### 旧版 Completions

当前支持字符串 prompt 或单项字符串数组，并要求 `n=1`、`best_of=1`。不支持 token ID
prompt、多候选结果或完整 logprobs 语义；响应中的 `logprobs` 为 `null`。采样、penalty、
suffix、seed、user 等传统 Completions 控件不会转发。

### token 估算

Anthropic 与 Gemini token-count 路径使用本地 `cl100k_base` tokenizer，对转换后的 Codex
input、工具 schema 和工具结果进行估算。结果适用于预检和预算，不保证与供应商 tokenizer
逐 token 一致，也不应用于账单核对。这两个路径仍要求下游 API Key，但不要求有效 OAuth 或
访问 ChatGPT 上游。

## 8. 正文、流与 WebSocket

- Live/Realtime multipart bootstrap 上限为 16 MiB；
- 图片、Realtime 和其他透明代理正文使用原生异步字节流；
- SSE 转换按事件增量处理，并保留下游背压；
- Responses WebSocket 转发 close code、close reason 和协商后的子协议；
- 上游拒绝 WebSocket 握手时，服务返回经过安全 header 过滤的 HTTP 响应。

浏览器原生 `WebSocket` API 不能设置本项目要求的 API-key header。浏览器场景应使用受控的
HTTP bootstrap 与临时凭据，或由可信后端建立 sideband WebSocket；不得把长期 API Key 写入
URL 或自定义子协议。

连接数、正文和 WebSocket 限制由本服务所在主机及前置反向代理共同决定。需要代理 WebSocket
时，前置代理必须允许 HTTP Upgrade，并避免缓冲 SSE 响应。

## 9. 错误、CORS 与缓存

- 本地路由和已注册协议路径的错误方法、无效下游 API Key 返回空正文 `404`；
- 未注册路径透明转发；
- 服务生成的协议错误分别使用 OpenAI、Anthropic 或 Google envelope；
- 只有已确认的公开 API 响应添加 CORS header；管理响应不添加 CORS；
- 公开协议 API 和管理响应使用 `Cache-Control: no-store`；透明转发保留上游响应
  header；
- 公开协议 API 过滤客户端凭据、Cookie 和账户 ID；透明转发仅对以 `/backend-api/codex/` 开头的
  路径按许可配置处理认证 header，其他路径保持原始凭据；
- 透明转发得到的最终响应在 `Content-Type` 为 `text/html` 或 `application/xhtml+xml` 时保留状态与
  无关 header，但移除正文长度、编码和正文；本地状态页与管理页不适用这条规则。其他媒体类型
  保持原始正文。

默认 `server.cors_origin` 为 `*`，当前配置只支持一个原样的 origin 值，不实现动态 allowlist，也不
启用 credentialed CORS。

## 10. 透明路径的兼容边界

服务不解析未注册路径和 `/backend-api/*` 的协议正文，也不保证上游对相应 action 的
稳定性、可用性、鉴权方式或响应格式。UDP RTP/RTCP 等非 HTTP 能力不在转发范围内。

## 11. Web 控制台与登录会话

管理员和下游用户使用同一个入口 `/admin`。`/admin` 前缀下的所有请求都在本地处理，绝不会转发到
ChatGPT；未知路径或方法返回空 `404`。

| 路径 | 行为 |
| --- | --- |
| `GET/HEAD /admin` | 返回 React 控制台，每次请求注入新的 CSP nonce，`Cache-Control: no-store` |
| `GET/HEAD /admin/` | `308` 跳转到 `/admin` |
| `GET/HEAD /admin/assets/*` | 从 `frontend.dir/assets` 读取带指纹的静态资源，`immutable` 缓存 |
| `GET /admin/api/session` | 返回当前会话；未登录时为 `{"session": null}` |
| `POST /admin/api/session` | 登录，JSON 正文 `{"credential": "..."}` |
| `DELETE /admin/api/session` | 退出登录：撤销服务端会话并清除 Cookie，返回 `204` |
| `GET /admin/api/account?range=24h` | 用户会话查看自己的用量 |
| `/admin/api/<管理端点>` | 仅管理员会话，见第 13 节 |
| `GET/HEAD /` | `302` 跳转到 `/admin` |

`frontend.dir` 缺少 `index.html` 或文件不含 nonce 占位符时，`/admin` 返回本地 `503`。

### 登录识别

登录接口依次比较 `admin.secret`、已启用的 API Key 和已启用下游账户的 `account_id`（去除首尾
空白），命中管理员即获得管理员会话，命中后两者获得只读用户会话。同一字符串同时配置为多种身份
（含已停用记录）时拒绝登录并记录错误日志；创建或修改 API Key、下游账户时也会拒绝与
`admin.secret` 相同的值。失败统一返回 `401 invalid_credentials`，不透露命中了哪类记录。

`server.public_origin` 为非回环地址的 HTTP 时，管理员登录返回 `403 insecure_admin_origin`，除非
配置 `session.allow_insecure_http = true`；用户登录不受此限制。

同一客户端 10 分钟内失败 10 次后，登录接口返回 `429` 并带 `Retry-After`，期间正确凭据同样被拒绝。
客户端地址取 TCP 对端；只有对端属于 `server.trusted_proxies` 时，才从右向左读取
`X-Forwarded-For` 并跳过受信任代理。IPv6 按 `/64` 计数。

### 会话

会话保存在 `session.database_path` 指定的 SQLite 中。Cookie 只包含 32 字节随机 token，数据库只
保存其 SHA-256。会话不保存登录凭据，而是保存凭据的 HMAC 绑定值（密钥为自动生成的
`state.session_binding_key`），每个请求都会重新确认身份仍存在、已启用且凭据未变。

| 身份 | 空闲过期 | 绝对过期 |
| --- | --- | --- |
| 管理员 | 24 小时 | 7 天 |
| API Key / 下游账户 | 30 天 | 90 天 |

已登录请求距上次续期超过 5 分钟时，服务端顺延空闲期限（不超过绝对期限）并重新下发 Cookie；
token 不轮换。每个身份最多保留 20 个会话，超出时删除最旧的会话。以下情况会使会话失效：

- 退出登录；
- 修改 `admin.secret`、API Key 值或下游账户 `account_id`；
- 停用或删除 API Key、下游账户（重新启用后旧会话不会恢复，需要重新登录）；
- 更换 `state.session_binding_key`（全部会话失效）。

`public_origin` 为 HTTPS 时 Cookie 为 `__Host-codex-session`（`Path=/; Secure; HttpOnly;
SameSite=Lax`）；HTTP 时为 `codex-session`（`Path=/admin; HttpOnly; SameSite=Lax`）。转发到上游的
请求会移除这两个 Cookie 及旧版管理 Cookie。

所有 `/admin/api` 写请求（含登录、退出）必须带有与 `server.public_origin` 完全一致的 `Origin`，
登录还要求 `Content-Type: application/json`；控制台不响应 CORS。`/admin/api` 响应均为
`Cache-Control: no-store`。

### 用户用量

`GET /admin/api/account?range=24h` 的查询身份只取自会话，不接受客户端指定。`range` 支持 `24h`、
`7d`、`30d` 和 `all`。返回身份类型、请求次数、Token、成本、时间序列、模型占比与额度时间条，
但不会回显 account id、API Key、OAuth、邮箱或 Cookie。管理员会话调用返回 `403`。

额度根据该调用身份当前映射的单个账户或账户组可用成员读取，不会推进组的轮询游标；无法读取额度
时仍返回 Token 用量，`quota` 为 `null`。未分配路由的身份仍可查询既有 Token 记录，但没有账户额度。

### 旧路径

配置了已弃用的 `admin.path` 时，`GET /<admin.path>/admin` 返回 `308` 跳转到 `/admin`，该路径族下的
其他请求返回 `410`。旧的 `POST /account/data` 和 `/assets/*` 同样返回本地 `410`。

## 12. Token 用量统计

API Key 鉴权的 Codex Responses 请求，以及代理账户转发的 Codex Responses 请求，每次上游尝试
恰好写入一条 SQLite 记录，失败的请求同样记录。HTTP 请求是一次尝试；WebSocket 连接中每个
`response.create`/`response.append` 各是一次尝试，相同 response ID 只落库一次。`status` 取值：

- `completed`、`incomplete`：上游返回 `response.completed`/`response.incomplete`；
- `failed`：上游返回非 2xx、`response.failed` 或 `error` 事件，或路由在上游响应前出错（如凭据失效、上游不可达）；
- `cancelled`：客户端在终止事件前断开，或在 WebSocket 中开始了下一个 response（`errorCode` 为 `superseded`）；
- `interrupted`：上游流在终止事件前结束或出错（`errorCode` 为 `stream_truncated`）。

每条记录还包含：客户端请求的模型（`requestedModel`）与上游报告的模型（`model`，缺失时回退到
请求模型，计价按此字段）、是否流式、HTTP 状态码与错误码、端到端耗时与首个输出增量的耗时
（`latencyMs`、`ttftMs`）、客户端 IP（仅在 TCP 对端属于 `trusted_proxies` 时采信
`X-Forwarded-For`）、原始 `X-Forwarded-For`、`User-Agent`，以及 Codex 会话信息。会话 ID 取自
`session_id`/`thread_id` 请求头或 `x-codex-turn-metadata`，缺失时回退到请求体的
`prompt_cache_key`；`sessionKind` 为 `main`、`subagent`、`fork` 或 `compaction`（依据
`x-codex-parent-thread-id`、`x-openai-subagent`、分叉来源及 `/compact` 端点判断）。

数据库表结构版本记录在 SQLite `user_version` 中；版本不一致的旧表会在启动时被删除并重建。

管理员会话可读取 `GET /admin/api/usage?range=7d`。`range` 支持 `cycle`、`24h`、`7d`、
`30d` 和 `all`，省略时使用 `7d`。下游筛选使用 `downstreamType` 与 `downstreamId`，类型支持
`api_key` 和 `auth_proxy`；上游筛选使用 `upstreamType` 与 `upstreamId`，类型支持
`codex_account` 和 `account_group`。两个维度可以单独使用，也可以同时使用；同时使用时按逻辑与
重新计算全部聚合结果。ID 使用 `/state` 返回的稳定记录 ID，每组类型与 ID 参数必须一起出现。

`cycle` 只用于单个 `codex_account` 上游，且必须同时传入从该账户周额度窗口计算出的 `startAt`
和 `endAt`；账户组不提供“当前周期”。非法范围或筛选返回 `400`。返回 JSON 包含：

- `startAt`、`endAt` 和所选 `range`；
- `totals`：请求数以及输入、缓存命中、缓存创建、输出、推理和总 Token；
- `series`：按小时或天填充的趋势时间桶；`successfulRequests` 统计 `completed`/`incomplete`，`failedRequests` 统计 `failed`/`interrupted`，`cancelled` 不计入两者；
- `models`、`identities`：按模型及 API Key/代理账户聚合；
- `reasoningEfforts`、`serviceTiers`：按思考强度（`reasoning.effort`）及服务等级聚合，`value` 取自实际发往上游的请求体；未显式设置思考强度时为 `default`，未设置服务等级时为 `auto`（由上游决定），`priority` 即 Codex 的 Fast 模式，空字符串表示请求在读取请求体之前就失败了；
- `recentEvents`：最多 50 条事件，包含上述全部字段、身份与路由目标、端点、HTTP/WebSocket、思考强度、请求与响应服务等级（`reasoningEffort`、`serviceTier`、`responseServiceTier`）及 Token 明细。

缓存与推理 Token 分别是输入与输出 Token 的子集。该接口不返回 Key/OAuth，也不会记录或返回
请求正文与模型输出。

## 13. 管理 API

管理 JSON API 位于 `/admin/api`，需要管理员会话：未登录返回 `401 invalid_session`，用户会话返回
`403 admin_required`。控制台页面通过 `/admin?page=usage`、`?page=settings`、`?page=api-keys`、
`?page=accounts` 和 `?page=account` 切换视图。以下为相对 `/admin/api` 的端点：

| 方法与相对路径 | 用途 |
| --- | --- |
| `GET /state` | 读取 Codex 账户、账户组、路由、API Key 和下游账户 |
| `GET /codex-accounts/subscription?id=<id>` | 实时读取指定 Codex 账户订阅与额度 |
| `POST /codex-accounts/oauth/device` | 为新 Codex 账户创建设备授权请求 |
| `POST /codex-accounts/oauth/device/poll` | 轮询设备授权，并在成功后加入统一账户池 |
| `POST /codex-accounts/import` | 通过 JSON 或手动凭据添加 Codex 账户，无需设备登录 |
| `PUT /codex-accounts` | 更新账户名称或启用状态；禁用会释放直连路由 |
| `DELETE /codex-accounts` | 删除账户、OAuth、组成员关系和直连路由 |
| `GET /account-routing` | 读取账户组与调用身份路由 |
| `PUT /account-routing` | 原子替换经校验的账户组与路由配置 |
| `GET /usage?range=7d&upstreamType=codex_account&upstreamId=<id>&downstreamType=api_key&downstreamId=<id>` | 读取 SQLite 用量，可分别或组合筛选上游与下游 |
| `GET /pricing` | 读取模型价格与已使用模型 |
| `PUT /pricing` | 替换模型价格配置 |
| `POST /pricing/sync` | 从价格源同步模型价格 |
| `GET /api-keys` | 读取 API Key 列表 |
| `POST /api-keys` | 创建 API Key |
| `PUT /api-keys` | 更新名称、值或启用状态 |
| `DELETE /api-keys` | 删除 API Key |
| `POST /auth-proxy` | 创建下游账户，默认不分配路由 |
| `PUT /auth-proxy` | 更新下游账户的名称、`account_id` 或启用状态 |
| `DELETE /auth-proxy` | 删除下游账户并释放其路由 |

所有管理写请求必须带有与 `server.public_origin` 完全一致的 `Origin`。停用或删除 API Key、下游
账户前会先撤销其全部控制台会话；撤销失败时返回 `503` 且不修改配置。

Codex 账户、API Key、下游账户和账户组使用 UUID 格式的稳定 `id`。禁用 Codex 账户会暂停其路由，
删除账户会同时移除组成员关系和直连路由。

### 凭据导入

`POST /codex-accounts/import` 接受单个账户的 JSON，正文上限为 64 KiB。此接口使用相同的管理会话
和同源校验。既可直接提交 Token 对象 / `auth.json`，也可使用 `{ "name": "显示名称", "credentials": {...} }`
包装。示例：

```json
{
  "name": "主账户",
  "credentials": {
    "tokens": {
      "access_token": "<access token>",
      "refresh_token": "<refresh token>",
      "id_token": "<id token>",
      "account_id": "<ChatGPT account ID>"
    }
  }
}
```

- `refresh_token` 必填，用于补齐凭据和后台续期；仅提交 `{ "refresh_token": "<refresh token>" }` 也可导入。
- `access_token`、`id_token`、`account_id`、`email` 可选。Account ID 和邮箱会从 JWT 中提取；无法识别
  Account ID 时返回 `400`，需要显式填写 `account_id`。此 ID 是上游账户 ID，不是 Router 生成的记录 UUID。
- `expires_at` 可选，支持 Unix 秒 / 毫秒时间戳（整数或数字字符串）和 RFC 3339 时间；也支持 `expires_in`
  正数秒数。提供的到期时间不会延长 JWT 本身的有效期。Access Token 缺失、过期或有效期未知时会先刷新，
  刷新失败不创建账户。
- Token 字段既支持蛇形命名，也支持 `accessToken`、`refreshToken`、`idToken`、`accountId`、`expiresAt`
  和 `expiresIn`。使用 `tokens` 包装时，Token 和到期时间字段应放在该对象内。
- `name` 可选，留空时使用邮箱或自动生成名称。成功返回 `201 { "account": {...} }`，账户立即加入现有
  账户池；响应只包含名称、ID、启用状态、OAuth 元数据和订阅元数据，不回传 Token。
- 重复账户返回 `409 codex_account_conflict`；字段错误返回 `400 invalid_credential_import`，JSON 语法
  错误返回 `400 invalid_json`。格式检查不代表 Token 已通过上游鉴权，实际有效性以刷新或上游请求结果为准。

凭据与账户目录在一次配置写入中保存。此接口不支持仅含 `OPENAI_API_KEY` 的 API Key 登录文件；
下游 API Key 仍通过“API Key”页面单独管理。
