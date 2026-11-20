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

明确注册的公开协议 API 要求一个已启用用户的已启用 API Key；健康检查、已知 API 的预检请求、管理
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
| `GET /v1/models` | 协议转换 | 读取 Codex 模型目录并补充内置图像模型，返回 OpenAI model list |
| `POST /v1/chat/completions` | 协议转换 | Chat 请求转换为 Responses；返回 Chat JSON 或 SSE |
| `POST /v1/completions` | 协议转换 | legacy prompt 转换为 Responses；返回 `text_completion` JSON 或 SSE |
| `POST /v1/responses` | 原生映射 | 映射到 `/backend-api/codex/responses` 并应用 Responses 创建策略 |
| `POST /v1/responses/compact` | 原生映射 | 映射到 Codex compact，仅应用 input 角色规范化 |
| `GET /v1/responses` + WebSocket Upgrade | 原生映射 | 建立 Responses 双向 WebSocket bridge |
| `/v1/responses/*` 其他子路径 | 透明传输 | 映射到同名 Codex 子路径；上游决定是否支持 |
| `POST /v1/images/generations` | 原生映射 | JSON 请求映射到 Codex Images，支持 Base64、Data URL 和 SSE |
| `POST /v1/images/edits` | 协议转换 | 支持 JSON 与 multipart，将上传的图片和 mask 转换为 Codex JSON |
| `/v1/images[/…]` 其他路径 | 透明传输 | 映射到同名 Codex Images 路径；上游决定是否支持 |
| `/v1/alpha/search` | 透明传输 | 映射到 `/backend-api/codex/alpha/search` |
| `POST /v1/live` | 原生映射 | 映射到 Codex Realtime call bootstrap，并补充缺失的默认查询参数 |
| `POST /v1/realtime/calls` | 原生映射 | 与 `/v1/live` 使用相同的 bootstrap 逻辑 |
| `GET /v1/live/{call_id}` + WebSocket Upgrade | 透明传输 | 连接 `api.openai.com` 的 Live sideband |
| `GET /v1/realtime?call_id=…` + WebSocket Upgrade | 透明传输 | 校验 `call_id` 后连接 Realtime sideband |
| `GET /v1/realtime/calls/{call_id}` + WebSocket Upgrade | 透明传输 | 连接对应 Realtime sideband |
一般透明代理路径拒绝 `CONNECT`；`OPTIONS` 由预检逻辑处理。除表中明确限制方法的路径外，
路由层允许其他 HTTP 方法，上游能力仍由目标 action 决定。

### 图片生成与编辑

`POST /v1/images/generations` 和 `POST /v1/images/edits`（也接受尾部 `/`）分别调用
`/backend-api/codex/images/generations` 和 `/backend-api/codex/images/edits`。
两者要求非空 `prompt`；未指定 `model` 时不补默认值，由上游决定。显式模型名、`size`、`quality`、
`n`、`background`、`output_format` 和其他扩展字段会传给上游；不根据模型列表限制请求，
参数是否被接受或执行仍取决于上游。

编辑请求接受以下两种形式：

- `application/json`：使用 `images: [{"image_url":"…"}]` 或 `images: [{"file_id":"…"}]`；
  可选 `mask` 同样接受 `image_url` 或 `file_id`。
- `multipart/form-data`：上传一个或多个 `image` / `image[]` 文件，以及可选的一个 `mask` 文件。
  文件会编码为 Base64 Data URL，并转换成 `images[].image_url` / `mask.image_url`；
  `n`、`output_compression`、`partial_images` 等数字字段和 `stream` 布尔字段会转换为相应 JSON 类型。
  也可通过 `mask[file_id]` / `mask[image_url]` 指定已有 mask 引用。

`response_format` 默认是 `b64_json`，保留上游的标准 `data[].b64_json` 返回。
指定 `url` 时转换为 `data[].url`，内容是 `data:image/…;base64,…`，不是托管图片的 HTTP 地址。
`stream: true` 保留原生图像 SSE；`url` 模式按事件转换 `b64_json`，保留事件名、ID 和用量字段。
上游错误保留原始状态、正文和重试相关 header。

标准 `/v1/models` 只转换上游 Codex 模型目录的格式，不补充任何条目。带 `client_version`
查询参数的原生 Codex 模型目录直接返回上游内容。

### Backend API 与透明转发

路由按请求路径和方法选择，不按入站 Host 分流。`/backend-api` 路径族直接以原方法、路径、
Query、流式正文和端到端 header 转发到固定的 `https://chatgpt.com` 上游，并返回 HTTP、SSE 或
WebSocket 响应。配置 `upstream.chatgpt_proxy` 时这些连接通过 SOCKS5 建立。该路径族不执行下游
API Key 鉴权或协议转换。

仅路径以 `/backend-api/codex/` 开头，且请求中的 `ChatGPT-Account-ID` 精确匹配某个已启用用户的
已启用账户 ID 凭据时，服务查询该用户的账户路由。分配到单个账户或账户组时，会使用调度得到的有效
Codex OAuth 替换原请求中已有的 `Authorization` 和 `ChatGPT-Account-ID`；尚未分配、用户或凭据已停用
或未匹配时按原认证信息透明转发。已配置路由但目标没有可用账户时返回本地错误。
其他 `/backend-api` 路径（包括不带尾部斜杠的 `/backend-api/codex`）保留原始凭据，不查询账户路由。

账户组的 `strategy` 支持以下值：

- `round-robin`：按稳定账户顺序平均轮询；
- `weighted-round-robin`：按 Codex 额度窗口的平均
  `remainingPercent / 距离 resetAt 的剩余分钟` 平滑加权，未知额度按组内平均权重处理；
- `fallback`：按用户固定账户（同一用户的所有凭据共用），并在该账户不可用时切换。

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

### 模型名指定思考强度

所有生成类路由（Responses HTTP / WebSocket、compact、Chat Completions、旧版 Completions、
Anthropic Messages、Gemini Content）都接受 `模型(思考强度)` 形式的模型名，例如
`gpt-6-astra(high)`。此时上游收到的 `model` 为 `gpt-6-astra`，`reasoning.effort` 强制为
`high`，覆盖请求中原有的思考强度字段（`reasoning.effort`、`reasoning_effort`、Anthropic
`thinking`、Gemini `thinkingConfig` 等），`reasoning` 中的其他字段保持不变。括号内只允许字母、
数字、`-` 和 `_`，统一转为小写，具体取值是否有效由上游判断。用量统计记录的是去掉后缀的模型名和
实际生效的思考强度。

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
逐 token 一致，也不应用于账单核对。这两个路径仍要求用户 API Key，但不要求有效 OAuth 或
访问 ChatGPT 上游。

## 8. 正文、流与 WebSocket

- Live/Realtime multipart bootstrap 上限为 16 MiB；
- Images 生成/编辑 JSON 正文上限为 100 MiB，编辑 multipart 上传总大小上限为 64 MiB；
- Images 的 `url` 响应转换最多读取 100 MiB JSON，SSE 单事件上限为 100 MiB；
- Images 默认 Base64 响应、Realtime 和其他透明代理正文使用原生异步字节流；
- SSE 转换按事件增量处理，并保留下游背压；
- Responses WebSocket 转发 close code、close reason 和协商后的子协议；
- 上游拒绝 WebSocket 握手时，服务返回经过安全 header 过滤的 HTTP 响应。

浏览器原生 `WebSocket` API 不能设置本项目要求的 API-key header。浏览器场景应使用受控的
HTTP bootstrap 与临时凭据，或由可信后端建立 sideband WebSocket；不得把长期 API Key 写入
URL 或自定义子协议。

连接数、正文和 WebSocket 限制由本服务所在主机及前置反向代理共同决定。需要代理 WebSocket
时，前置代理必须允许 HTTP Upgrade，并避免缓冲 SSE 响应。

## 9. 错误、CORS 与缓存

- 本地路由和已注册协议路径的错误方法、无效 API Key 返回空正文 `404`；
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
| `GET /admin/api/account/analytics` | 用户会话的用量分析，参数同 `/usage/analytics`，固定限定为本人 |
| `GET /admin/api/account/events` | 用户会话的调用日志，参数同 `/usage/events`，固定限定为本人 |
| `GET /admin/api/account/quota` | 用户会话的身份摘要、路由与所在账户池的额度预测 |
| `/admin/api/<管理端点>` | 仅管理员会话，见第 13 节 |
| `GET/HEAD /` | `302` 跳转到 `/admin` |

`frontend.dir` 缺少 `index.html` 或文件不含 nonce 占位符时，`/admin` 返回本地 `503`。

### 登录识别

登录接口比较 `admin.secret` 与所有用户的凭据：API Key 按原值比较，账户 ID 去除首尾空白后比较。
命中管理员即获得管理员会话；命中某个用户已启用的凭据（且用户已启用）获得该用户的只读会话，
同一用户的任一凭据登录后看到的是同一个看板。凭据值在全部用户之间唯一；某个用户凭据（含已停用
记录）与 `admin.secret` 相同时拒绝登录并记录错误日志，创建或修改用户时也会拒绝这样的值。失败统一
返回 `401 invalid_credentials`，不透露命中了哪类记录。

`server.public_origin` 为非回环地址的 HTTP 时，管理员登录返回 `403 insecure_admin_origin`，除非
配置 `session.allow_insecure_http = true`；用户登录不受此限制。

同一客户端 10 分钟内失败 10 次后，登录接口返回 `429` 并带 `Retry-After`，期间正确凭据同样被拒绝。
客户端地址取 TCP 对端；只有对端属于 `server.trusted_proxies` 时，才从右向左读取
`X-Forwarded-For` 并跳过受信任代理。IPv6 按 `/64` 计数。

### 会话

会话保存在 `session.database_path` 指定的 SQLite 中。Cookie 只包含 32 字节随机 token，数据库只
保存其 SHA-256。会话不保存登录凭据，而是保存登录所用凭据的 HMAC 绑定值（密钥为自动生成的
`state.session_binding_key`），每个请求都会重新确认用户仍存在、已启用，且该凭据仍启用、值未变。

| 身份 | 空闲过期 | 绝对过期 |
| --- | --- | --- |
| 管理员 | 24 小时 | 7 天 |
| 用户 | 30 天 | 90 天 |

已登录请求距上次续期超过 5 分钟时，服务端顺延空闲期限（不超过绝对期限）并重新下发 Cookie；
token 不轮换。管理员与每个用户各最多保留 20 个会话，超出时删除最旧的会话。以下情况会使会话失效：

- 退出登录；
- 修改 `admin.secret`；
- 停用或删除用户，或修改、停用、删除用户已启用的凭据：该用户的全部会话失效（重新启用或改回原值后
  旧会话不会恢复，需要重新登录）；仅改名称、备注或新增凭据不影响已有会话；
- 更换 `state.session_binding_key`（全部会话失效）。

`public_origin` 为 HTTPS 时 Cookie 为 `__Host-codex-session`（`Path=/; Secure; HttpOnly;
SameSite=Lax`）；HTTP 时为 `codex-session`（`Path=/admin; HttpOnly; SameSite=Lax`）。转发到上游的
请求会移除这两个 Cookie 及旧版管理 Cookie。

所有 `/admin/api` 写请求（含登录、退出）必须带有与 `server.public_origin` 完全一致的 `Origin`，
登录还要求 `Content-Type: application/json`；控制台不响应 CORS。`/admin/api` 响应均为
`Cache-Control: no-store`。

### 用户用量

`GET /admin/api/account?range=24h` 的查询用户只取自会话，不接受客户端指定，统计范围是该用户的全部
凭据。`range` 支持 `24h`、`7d`、`30d` 和 `all`。返回请求次数、Token、成本、时间序列、模型占比与
额度时间条，但不会回显账户 ID、API Key、OAuth、邮箱或 Cookie。管理员会话调用返回 `403`。

额度根据该用户当前映射的单个账户或账户组可用成员读取，不会推进组的轮询游标；无法读取额度
时仍返回 Token 用量，`quota` 为 `null`。未分配路由的用户仍可查询既有 Token 记录，但没有账户额度。

`/account/analytics` 与 `/account/events` 拒绝 `downstreamType`/`downstreamId` 参数（`400`），
用户只取自会话。`/account/quota` 返回 `identity`（用户名称）、`route`、`group`（账户组名称与策略）
和 `visible`；仅当 `public_account.show_quota` 开启时才附带 `quota`（结构同管理端 `/quota`，仅含
该用户路由到的账户）。

### 旧路径

配置了已弃用的 `admin.path` 时，`GET /<admin.path>/admin` 返回 `308` 跳转到 `/admin`，该路径族下的
其他请求返回 `410`。旧的 `POST /account/data` 和 `/assets/*` 同样返回本地 `410`。

## 12. Token 用量统计

API Key 鉴权的 Codex Responses 请求，以及按账户 ID 转发的 Codex Responses 请求，每次上游尝试
恰好写入一条 SQLite 记录。记录包含用户（`userId`、`userName`）和本次使用的凭据（`credentialType`
为 `api_key` 或 `account_id`、`credentialId`、`credentialName`；未设备注时为值的末 4 位）。失败的请求同样记录。HTTP 请求是一次尝试；WebSocket 连接中每个
`response.create`/`response.append` 各是一次尝试，相同 response ID 只落库一次。`status` 取值：

- `completed` 和 `incomplete` 对应上游完成或提前结束的响应。`response.done` 按事件内的 `response.status` 记账，缺省为完成。
- `failed` 包括上游非 2xx、流中的 `error`、`response.error`、`response.failed`，以及收到上游响应前的路由错误。上游可以先返回 HTTP 200，再通过 SSE 报错。
- `cancelled` 包括转发在终止事件或上游读取结果到达前被释放（`client_cancelled`）、WebSocket 开始下一次响应（`superseded`），以及上游明确报告的取消状态。
- `interrupted` 表示响应缺少终止事件。读到流结尾时记为 `stream_truncated`，上游 HTTP 正文读取报错时记为 `upstream_read_error`。

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
`user`（用户的全部凭据）和 `credential`（单个凭据）；上游筛选使用 `upstreamType` 与 `upstreamId`，类型支持
`codex_account` 和 `account_group`。两个维度可以单独使用，也可以同时使用；同时使用时按逻辑与
重新计算全部聚合结果。ID 使用 `/state` 返回的稳定记录 ID，每组类型与 ID 参数必须一起出现。

`cycle` 只用于单个 `codex_account` 上游，且必须同时传入从该账户周额度窗口计算出的 `startAt`
和 `endAt`；账户组不提供“当前周期”。非法范围或筛选返回 `400`。返回 JSON 包含：

- `startAt`、`endAt` 和所选 `range`；
- `totals`：请求数以及输入、缓存命中、缓存创建、输出、推理和总 Token；
- `series`：按小时或天填充的趋势时间桶；`successfulRequests` 统计 `completed`/`incomplete`，`failedRequests` 统计 `failed`/`interrupted`，`cancelled` 不计入两者；
- `models`、`identities`：按模型及用户聚合；
- `reasoningEfforts`、`serviceTiers`：按思考强度（`reasoning.effort`）及服务等级聚合，`value` 取自实际发往上游的请求体；未显式设置思考强度时为 `default`，未设置服务等级时为 `auto`（由上游决定），`priority` 即 Codex 的 Fast 模式，空字符串表示请求在读取请求体之前就失败了；
- `recentEvents`：最多 50 条事件，包含上述全部字段、身份与路由目标、端点、HTTP/WebSocket、思考强度、请求与响应服务等级（`reasoningEffort`、`serviceTier`、`responseServiceTier`）及 Token 明细。

缓存与推理 Token 分别是输入与输出 Token 的子集。该接口不返回 Key/OAuth，也不会记录或返回
请求正文与模型输出。

### 用量分析与调用日志

`GET /admin/api/usage/analytics` 在一次扫描中计算控制台各分析页所需的全部聚合。时间参数：

- `range`：`24h`、`7d`（默认）、`30d`、`all` 或 `custom`；`custom` 需要 `startAt`、`endAt`
  （毫秒，跨度不超过 400 天，`endAt` 最多超前当前时间 2 天，便于按自然日查询“今天”）；
- `bucket`：时间桶分钟数，默认 2 天内 60、8 天内 360、更长 1440；桶数不超过 2000。

筛选参数（均可组合，多值参数可重复或用逗号分隔）：`downstreamType`/`downstreamId`、
`upstreamType`/`upstreamId`（同 `/usage`）、`model`、`status`、`effort`、`tier`（请求的服务等级：
`priority`、`default`、`auto`、`flex`）、`transport`、`sessionKind`、`sessionId`（同时匹配父会话）
和 `q`（在 request ID、response ID、session ID 中不区分大小写搜索）。返回：

- `totals`：结果计数（`completed`、`incomplete`、`failed`、`cancelled`、`interrupted`、`rateLimited` 为 HTTP 429）、Token 与成本、未定价 Token、Fast 请求数及其未乘倍率的成本、会话数、`ttft`/`latency` 的 P50/P95/最大值（仅统计成功请求），以及输出速度中位数（`outputTokens × 1000 ÷ latencyMs`）；
- `previous`：等长的上一周期、截至相同经过时间的 `totals`（`all` 为 `null`）；
- `series`：按桶的结果计数、Token 构成、成本、TTFT/延迟分位数和 Fast 降级数；
- `models`、`identities`、`upstreamAccounts`、`upstreamGroups`：按模型、用户、Codex 账户、账户组聚合，含分位数、7 段 Token 趋势、429 按桶分布、会话保持率与使用方；
- `identityModels`、`effortTiers`：身份 × 模型、思考强度 × 实际服务等级的交叉矩阵；
- `fast`：Fast 请求、兑现与降级次数，按账户和模型细分；
- `errors`：按状态、HTTP 状态码、错误码分组的未完成请求，含最近时间与涉及账户；
- `sessions`、`clients`、`transports`：会话类型与最近会话、按 User-Agent 解析的客户端与 IP、传输方式/流式/端点对比；
- `latencyDensity`：TTFT × 总延迟的对数分箱计数；`unpricedModels`；`recentFailures`（最近 20 条）。

`GET /admin/api/usage/events` 使用相同的时间与筛选参数，另支持：

- `view`：`all`（默认）、`failed`（`failed`/`interrupted`）、`slow`（总延迟超过区间内成功请求的 P95）、
  `downgraded`（请求 Fast 但上游返回其他等级）、`subagent`、`rate_limited`（HTTP 429）；
- `offset`、`limit`（1–500，默认 50），按时间倒序分页；
- `group=session`：按会话树分组，`total` 为组数，`sessions` 每组含根会话的最新请求和子会话摘要；
  无会话的请求按用户成组；
- `format=csv`：导出匹配的请求（最多 50 000 条），字段含全部记录列与成本，公式前缀会被转义。

分页响应还包含区间内未筛选的总数 `rangeTotal`、匹配请求的 `summary` 与 `series`、各视图计数
`views` 和慢请求阈值 `slowThresholdMs`。

## 13. 管理 API

管理 JSON API 位于 `/admin/api`，需要管理员会话：未登录返回 `401 invalid_session`，用户会话返回
`403 admin_required`。控制台页面通过 `/admin?page=<页面>` 切换视图，页面为 `logs`、`usage`、
`performance`、`codex`、`groups`、`users`、`pricing` 和 `settings`（旧的
`account`、`routing`、`accounts`、`api-keys`、`downstream` 会映射到新页面）。以下为相对 `/admin/api` 的端点：

| 方法与相对路径 | 用途 |
| --- | --- |
| `GET /state` | 读取 Codex 账户、账户组、路由和用户 |
| `GET /codex-accounts/subscription?id=<id>` | 实时读取指定 Codex 账户订阅与额度 |
| `POST /codex-accounts/oauth/device` | 为新 Codex 账户创建设备授权请求 |
| `POST /codex-accounts/oauth/device/poll` | 轮询设备授权，并在成功后加入统一账户池 |
| `POST /codex-accounts/import` | 通过 JSON 或手动凭据添加 Codex 账户，无需设备登录 |
| `PUT /codex-accounts` | 更新账户名称或启用状态；禁用会释放直连路由 |
| `DELETE /codex-accounts` | 删除账户、OAuth、组成员关系和直连路由 |
| `GET /account-routing` | 读取账户组与用户路由 |
| `PUT /account-routing` | 原子替换经校验的账户组与路由配置 |
| `GET /usage?range=7d&upstreamType=codex_account&upstreamId=<id>&downstreamType=user&downstreamId=<id>` | 读取 SQLite 用量，可分别或组合筛选上游与下游 |
| `GET /usage/analytics` | 用量分析聚合，见第 12 节 |
| `GET /usage/events` | 分页调用日志、会话分组与 CSV 导出，见第 12 节 |
| `GET /quota` | 各账户缓存的额度快照与耗尽预测；`?refresh=1` 先立即采样全部启用账户 |
| `GET /pricing` | 读取模型价格、已使用模型与上次同步状态 `sync` |
| `PUT /pricing` | 替换模型价格配置 |
| `POST /pricing/sync` | 从价格源同步模型价格，并记录同步状态 |
| `GET /settings` | 读取用户端、上游、计费和通知设置；`upstream.codexClientVersion` 为模型目录默认版本 |
| `PUT /settings` | 保存完整设置对象，`upstream.codexClientVersion` 使用数字 `x.y.z` 格式，保存后即时生效 |
| `POST /settings/notifications/test` | 通过已保存并启用的通道发送测试通知，正文 `{"channel": "bark"}` 或 `"dingtalk"`；通道未启用返回 `409` |
| `GET /users` | 读取用户及其凭据 |
| `POST /users` | 创建用户，默认不分配路由 |
| `PUT /users` | 整体更新用户的名称、启用状态与凭据列表 |
| `DELETE /users` | 删除用户及其全部凭据，并释放其路由 |

用户正文形如：

```json
{
  "id": "<更新时必填>",
  "name": "张三",
  "enabled": true,
  "credentials": [
    { "id": "<已有凭据的 id，新凭据省略>", "type": "api_key", "value": "sk-…", "name": "laptop", "enabled": true },
    { "type": "account_id", "value": "<ChatGPT 账户 ID>", "name": "", "enabled": true }
  ]
}
```

每个用户需要 1 到 20 个凭据。API Key 需 11–512 个字符并包含字母、数字与符号；账户 ID 为 1–256 个
可见 ASCII 字符，保存前去除首尾空白。凭据值在全部用户和类型之间唯一。`PUT` 时与已有凭据 `id`
匹配的条目保留原 `id`，其余条目分配新 `id`。

所有管理写请求必须带有与 `server.public_origin` 完全一致的 `Origin`。若更新停用了用户，或修改、
停用、删除了其已启用的凭据，或删除用户，会先撤销该用户的全部控制台会话；撤销失败时返回 `503`
且不修改配置。

Codex 账户、用户、凭据和账户组使用 UUID 格式的稳定 `id`。禁用 Codex 账户会暂停其路由，
删除账户会同时移除组成员关系和直连路由。

### 额度预测

`GET /quota` 读取后台定期采样的额度快照，不会请求上游（`refresh=1` 除外）。每个窗口附带：
窗口起点（`resetAt` 减窗口长度）以来该账户的请求、Token 与成本；已用比例不低于 1% 时的每 1%
对应 Token 与成本；按最近 3 小时 Token 消耗折算的每小时消耗比例（无用量时退回窗口内平均速度）；
预计耗尽时间及是否早于重置。`groups` 给出各账户组成员在当前额度下承接新请求的比例，与路由
实际使用的权重一致；`quarantined` 表示账户因 429 或额度耗尽暂时退出路由。

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
下游 API Key 仍在“用户”页面管理。
