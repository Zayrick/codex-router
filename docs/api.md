# API 与协议兼容性

## 1. 兼容范围

Codex Router 通过协议转换、Codex 原生映射和透明传输提供多种客户端接口。本文描述的是当前
仓库代码与测试所保证的行为；具体模型、额度和私有 Codex action 是否可用，仍取决于 OpenAI
账户、ChatGPT relay 和上游服务。

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

同时提供多个 header 时，服务只验证优先级最高的值，不会在验证失败后回退。缺失、错误或
已停用的 Key 返回空正文 `404`。

已知公开路径的 `OPTIONS` 请求无需鉴权，返回 `204` 并应用 `server.cors_origin`。管理路由不启用
CORS。

### 健康检查

| 方法与路径 | 鉴权 | 行为 |
| --- | --- | --- |
| `GET /healthz` | 无 | 配置文件中的 OAuth 可读取且未过期时返回空正文 `204`；其他情况返回空正文 `404` |

健康检查不验证 relay 可达性，也不发起上游请求。

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
| `GET /v1/live/{call_id}` + WebSocket Upgrade | 透明传输 | 直连 `api.openai.com` 的 Live sideband |
| `GET /v1/realtime?call_id=…` + WebSocket Upgrade | 透明传输 | 校验 `call_id` 后直连 Realtime sideband |
| `GET /v1/realtime/calls/{call_id}` + WebSocket Upgrade | 透明传输 | 直连对应 Realtime sideband |
一般透明代理路径拒绝 `CONNECT`；`OPTIONS` 由预检逻辑处理。除表中明确限制方法的路径外，
路由层允许其他 HTTP 方法，上游能力仍由目标 action 决定。

### Backend API 与透明转发

路由按请求路径和方法选择，不按入站 Host 分流。`/backend-api` 路径族直接以原方法、路径、
Query、流式正文和端到端 header 转发到 `CHATGPT_RELAY_URL`，并返回 relay 的 HTTP、SSE 或
WebSocket 响应。该路径族不执行下游 API Key 鉴权或协议转换。

`/backend-api` 请求中的 `ChatGPT-Account-ID` 精确匹配一条已启用代理账户记录时，服务优先
使用该记录自己的有效 OAuth；该记录尚未登录、Token 已过期或凭据缺少账户 ID 时自动回退到主
Codex OAuth。两者都会替换请求中已有的 `Authorization` 和 `ChatGPT-Account-ID`。记录已停用或
未匹配时按原认证信息转发。

未注册的其他 HTTP 路径同样透明转发到 relay，并保留 `Authorization`、
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
relay。

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

服务不解析未注册路径和 `/backend-api/*` 的协议正文，也不保证 relay 对相应 action 的
稳定性、可用性、鉴权方式或响应格式。UDP RTP/RTCP 等非 HTTP 能力不在转发范围内。

## 11. 订阅额度状态

后台维护任务按 `server.maintenance_interval_seconds` 周期采集用量，并把快照写回配置文件。
浏览器请求路径不会实时访问 Codex 上游。

`GET /status/usage/data` 返回公开快照字段：采样时间、订阅类型，以及每个窗口的 ID、
类别、名称、周期类型、已用/剩余百分比、窗口秒数和重置时间。它不返回 OAuth、账户 ID、邮箱、
API Key、Cookie、管理信息或内部告警投递状态。尚未完成首次采样时返回空快照；读取失败时返回
`503`。该路径精确匹配且只接受 `GET`；其他方法返回空 `404`。

`GET /status/usage` 返回公开 React 用量页面。该页面只读取上述公开快照接口，不读取或展示
OAuth、API Key、管理配置或管理会话。页面路径同样精确匹配，其他方法返回空 `404`。

## 12. Token 用量统计

API Key 鉴权的 Codex Responses 请求，以及代理账户转发的 Codex Responses 请求，会在收到上游
终止 JSON/SSE/WebSocket 事件时把用量写入 SQLite。一个 WebSocket 连接可以记录多个 response，
相同 response ID 只落库一次。模型优先取终止响应，缺失时使用对应请求中的模型。

管理会话可读取 `GET /<admin.path>/admin/usage?range=7d`。`range` 支持 `24h`、`7d`、`30d` 和
`all`，省略或传入未知值时使用 `7d`。返回 JSON 包含：

- `startAt`、`endAt` 和所选 `range`；
- `totals`：请求数以及输入、缓存命中、缓存创建、输出、推理和总 Token；
- `series`：按小时或天填充的趋势时间桶；
- `models`、`identities`：按模型及 API Key/代理账户聚合；
- `recentEvents`：最多 50 条事件，包含模型、身份、端点、HTTP/WebSocket、状态与 Token 明细。

缓存与推理 Token 分别是输入与输出 Token 的子集。该接口不返回 Key/OAuth，也不会记录或返回
请求正文与模型输出。连接提前中断、未收到终止事件时不会生成统计；失败终止事件即使没有 usage
也会以零 Token 事件记录，便于在明细中观察失败请求。

## 13. 管理 API

管理 JSON API 位于 `/<admin.path>/admin`。精确的 `GET /<admin.path>/admin` 返回 React 管理页面；
错误方法、额外路径段和其他隐藏路径族请求返回空 `404`。页面与以下 JSON 端点共享管理契约：

| 方法与相对路径 | 用途 |
| --- | --- |
| `POST /login` | 使用 `admin.secret` 创建管理会话 |
| `POST /logout` | 清除管理会话 |
| `GET /state` | 读取 OAuth 摘要、订阅摘要、API Key 列表和 Backend API 代理设置 |
| `GET /subscription` | 实时读取订阅与额度 |
| `GET /usage?range=7d` | 读取 SQLite 中的下游 Token 用量聚合与最近事件 |
| `POST /oauth/device` | 创建设备授权请求 |
| `POST /oauth/device/poll` | 轮询设备授权结果 |
| `DELETE /oauth` | 删除已保存的 OAuth 凭据 |
| `GET /api-keys` | 读取 API Key 列表 |
| `POST /api-keys` | 创建 API Key |
| `PUT /api-keys` | 更新名称、值或启用状态 |
| `DELETE /api-keys` | 删除 API Key |
| `POST /auth-proxy` | 创建代理账户 |
| `PUT /auth-proxy` | 更新代理账户的名称、`account_id` 或启用状态 |
| `DELETE /auth-proxy` | 删除代理账户 |
| `POST /auth-proxy/oauth/device` | 为指定代理账户创建设备授权请求 |
| `POST /auth-proxy/oauth/device/poll` | 轮询指定代理账户的设备授权结果 |
| `DELETE /auth-proxy/oauth` | 删除指定代理账户的独立 OAuth 凭据 |

`/state`、`/subscription`、OAuth、API Key 和 Backend API 代理端点需要有效的管理会话；登录、退出
以及所有受保护的管理写请求必须带有与 `server.public_origin` 完全一致的 `Origin`。管理 Cookie
保持 `Secure`、`HttpOnly` 和 `SameSite=Strict`，因此浏览器管理流量应通过 HTTPS。

服务在创建 API Key 和代理账户时分配 UUID 格式的 `id`。更新、删除和代理账户 OAuth 请求
使用该 `id` 定位记录。
