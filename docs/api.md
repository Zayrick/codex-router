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

`/backend-api` 请求中的 `ChatGPT-Account-ID` 精确匹配一条已启用下游账户记录时，服务查询该
调用身份的账户路由。分配到单个账户或账户组时，会使用调度得到的有效 Codex OAuth 替换来访
`Authorization` 和 `ChatGPT-Account-ID`；尚未分配、记录已停用或未匹配时按原认证信息透明转发。
已配置路由但目标没有可用账户时返回本地错误。

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
`ChatGPT-Account-ID`、Cookie 和其他端到端 header。健康检查、状态页、隐藏管理路径和已注册
协议 API 按各自的本地路由处理。

当 `/v1/models` 包含 `client_version` 查询参数时，服务保留 Codex CLI 模型目录格式，而
不转换为 OpenAI model list。

## 4. Anthropic 路由

| 方法与路径 | 级别 | 行为 |
| --- | --- | --- |
| `POST /v1/messages` | 协议转换 | Anthropic Messages 转换为 Codex Responses；返回 Message JSON 或命名 SSE 事件 |
| `POST /v1/messages/count_tokens` | 本地处理 | 对转换后的 Codex input 使用本地 tokenizer 估算 `input_tokens` |

Messages 支持 system、文本、图片、文档、thinking、客户端工具、工具结果、Web Search、
tool choice、usage 和 stop reason 等主要结构。请求必须包含 Anthropic 形式的 `max_tokens`，
但该值不会转发为 Codex 的输出上限。`temperature`、`top_p`、`top_k`、`stop_sequences`、
metadata 和 `cache_control` 也不提供等价的上游语义。

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
tool config、thinking 和 usage metadata。`generationConfig` 中仅 thinking level/budget 具有
对应转换；采样参数、候选数、停止序列以及输出 MIME/schema 不会转发为 Codex 语义。

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
`previous_response_id`、`generate` 和 `stream_options` 的会话语义。Chat Completions、旧版
Completions、Anthropic Messages 与 Gemini Content 转换后发往 Codex Responses 的 HTTP
请求也应用上述 HTTP 删除规则。

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
- 公开协议 API 过滤客户端凭据、Cookie 和账户 ID；透明转发仅在 `/backend-api` 路径族按许可
  配置处理认证 header，其他路径保持原始凭据；
- 透明转发得到的最终响应在 `Content-Type` 为 `text/html` 或 `application/xhtml+xml` 时保留状态与
  无关 header，但移除正文长度、编码和正文；本地状态页与管理页不适用这条规则。其他媒体类型
  保持原始正文。

默认 `server.cors_origin` 为 `*`，当前配置只支持一个原样的 origin 值，不实现动态 allowlist，也不
启用 credentialed CORS。

## 10. 透明路径的兼容边界

服务不解析未注册路径和 `/backend-api/*` 的协议正文，也不保证上游对相应 action 的
稳定性、可用性、鉴权方式或响应格式。UDP RTP/RTCP 等非 HTTP 能力不在转发范围内。

## 11. 公开账户用量页

`GET /` 返回不带管理侧栏的 React 账户查询页。页面使用与管理登录页一致的表单，用户输入一个已
启用的 API Key 或 account id 后，结果直接显示在当前页面。

页面通过 `POST /account/data` 读取聚合数据，请求正文使用
`application/x-www-form-urlencoded`，包含 `credential` 与 `range`。`range` 支持 `cycle`、`24h`、
`7d`、`30d` 和 `all`。返回身份类型、请求次数、Token、成本、时间序列、模型占比与额度时间条，
但不会回显 account id、API Key、OAuth、邮箱、Cookie 或管理会话。

额度根据该调用身份当前映射的单个账户或账户组可用成员读取。公开查询不会推进组的轮询游标；
无法读取额度时仍返回 Token 用量，`quota` 为 `null`。无效、已停用或未分配的 API Key 返回本地
通用的 `404` JSON 错误；未分配下游账户仍可查询既有 Token 记录，但没有账户额度。

## 12. Token 用量统计

API Key 鉴权的 Codex Responses 请求，以及代理账户转发的 Codex Responses 请求，会在收到包含
usage 的上游终止 JSON/SSE/WebSocket 事件时把用量写入 SQLite。一个 WebSocket 连接可以记录多个
response，相同 response ID 只落库一次。模型优先取终止响应，缺失且请求模型可用时使用请求模型。

管理会话可读取 `GET /<admin.path>/admin/usage?range=7d`。`range` 支持 `cycle`、`24h`、`7d`、
`30d` 和 `all`，省略时使用 `7d`。下游筛选使用 `downstreamType` 与 `downstreamId`，类型支持
`api_key` 和 `auth_proxy`；上游筛选使用 `upstreamType` 与 `upstreamId`，类型支持
`codex_account` 和 `account_group`。两个维度可以单独使用，也可以同时使用；同时使用时按逻辑与
重新计算全部聚合结果。ID 使用 `/state` 返回的稳定记录 ID，每组类型与 ID 参数必须一起出现。

`cycle` 只用于单个 `codex_account` 上游，且必须同时传入从该账户周额度窗口计算出的 `startAt`
和 `endAt`；账户组不提供“当前周期”。非法范围或筛选返回 `400`。返回 JSON 包含：

- `startAt`、`endAt` 和所选 `range`；
- `totals`：请求数以及输入、缓存命中、缓存创建、输出、推理和总 Token；
- `series`：按小时或天填充的趋势时间桶；
- `models`、`identities`：按模型及 API Key/代理账户聚合；
- `recentEvents`：最多 50 条事件，包含模型、身份、端点、HTTP/WebSocket、状态与 Token 明细。

缓存与推理 Token 分别是输入与输出 Token 的子集。该接口不返回 Key/OAuth，也不会记录或返回
请求正文与模型输出；未收到 usage 的请求不会生成统计。

## 13. 管理 API

管理 JSON API 位于 `/<admin.path>/admin`。精确的 `GET /<admin.path>/admin` 返回 React 管理页面；
页面通过 `?page=usage`、`?page=pricing`、`?page=api-keys`、`?page=accounts` 和 `?page=account`
切换视图。错误方法、额外路径段和其他隐藏路径族请求返回空 `404`。页面与以下 JSON 端点共享管理契约：

| 方法与相对路径 | 用途 |
| --- | --- |
| `POST /login` | 使用 `admin.secret` 创建管理会话 |
| `POST /logout` | 清除管理会话 |
| `GET /state` | 读取 Codex 账户、账户组、路由、API Key 和下游账户 |
| `GET /codex-accounts/subscription?id=<id>` | 实时读取指定 Codex 账户订阅与额度 |
| `POST /codex-accounts/oauth/device` | 为新 Codex 账户创建设备授权请求 |
| `POST /codex-accounts/oauth/device/poll` | 轮询设备授权，并在成功后加入统一账户池 |
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

除登录和退出外，管理端点需要有效会话；所有管理写请求必须带有与 `server.public_origin`
完全一致的 `Origin`。管理 Cookie
保持 `Secure`、`HttpOnly` 和 `SameSite=Strict`，因此浏览器管理流量应通过 HTTPS。

Codex 账户、API Key、下游账户和账户组使用 UUID 格式的稳定 `id`。禁用 Codex 账户会暂停其路由，
删除账户会同时移除组成员关系和直连路由。
