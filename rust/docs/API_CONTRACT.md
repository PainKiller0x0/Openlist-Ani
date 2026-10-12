# HTTP 与网页兼容合同

本文件约束外部行为。字段、默认值、类型和路由函数的完整索引见 `reference/api-inventory.json` 与 `reference/request-models.json`；Python参考文件在源码包中。
索引由AST只读提取，没有执行生产代码，也不是已验证的OpenAPI文档。开发代理必须补成机器校验的合同测试。

## 1. 通用规则

- 保留所有索引中的接口，不因为其中一些曾给assistant用就删掉基础REST API。
- JSON响应字段使用原snake_case；optional/null/空list区别保留。不得统一换成自定义 `{code,data}` 外层。
- 失败以原 `{detail: ...}` 合同为主；UI可以读取detail/message。metadata失败返回success:false和明确message，不能HTTP200后伪造task完成。
- 原Pydantic输入校验错误需要422、body形状兼容；业务错误保留原400/401/404/409。若加413资源上限，响应仍为JSON。
- 所有输入校验和响应编码必须通过实际HTTP测试；不能只验证serde结构能序列化。
- 首页及登录HTML保持no-store/no-cache；静态assets允许合理缓存，不把API或上传种子文件公开缓存。
- 未登录的私有API401；私有网页303到/login。登录、会话查询、退出及assets保持相同公开范围。

## 2. 登录

GET `/api/auth/session` -> `{"authenticated":bool,"username":string|null}`。
POST `/api/auth/login` body `{username,password}` -> `{success:true,authenticated:true,username}`；用户名允许trim，密码不trim。
失败401；POST `/api/auth/logout` -> `{success:true}`并删除Cookie。

环境变量名称保留：`OP_ANI_AUTH_USER`、`OP_ANI_AUTH_PASSWORD_HASH`、`OP_ANI_AUTH_SECRET`。
禁止将auth.env复制进reference/Git/日志。仅systemd在生产加载，fixture使用synthetic材料。

密码格式：`pbkdf2_sha256$iterations$salt_base64url$digest_base64url`，无padding也能解码；迭代范围10000..2000000，密码UTF-8 bytes，PBKDF2-HMAC-SHA256，恒时比较。
会话Cookie为`op_ani_session`，30天；payload=`username|unix_expiry|nonce`，token=`payload.signature_base64url`，signature是HMAC-SHA256(payload)。
secret优先独立SECRET变量，没有时兼容以PASSWORD_HASH字符串为secret。用户名必须等于配置用户名；过期/伪造拒绝。
Cookie Path=/、HttpOnly、SameSite=Lax；可信HTTPS代理下Secure=true。生产不能根据任意公网X-Forwarded-Proto信任代理。
旧stateless logout只移除浏览器Cookie，不保证服务器撤销已复制token；不要在测试/文档里谎称退出能让所有旧token失效。
如果实施全token撤销，必须作为额外明确变更，保留会话兼容，并增加持久撤销机制测试；本次不默认扩展。

## 3. 网页状态与设置

GET `/api/ui/state` 必须保留：
`rss_urls/rss_subscriptions/global_exclude_patterns/openlist_url/download_path/poll_interval_seconds/max_download_retries/mikan_base_url/rss_status/tasks`。
rss_urls仅active源；rss_subscriptions包含暂停/完结项，并保留原UI消费的海报、季、最新集数等字段。
rss_status必须包含真实扫描开始/完成/下一次时间以及running/enabled状态；不能定时伪造。

GET `/api/ui/settings`的llm包含`provider_type/base_url/model/api_key_configured/tmdb_language/metadata_parser_provider`，不返回API key。
POST `/api/ui/settings`保留原设置表单与optional语义：字段未提供不清空秘密/地址；空key如何处理以参考代码/合同样本验证。
修改路径、OpenList URL、rename_format前先验证；失败保持旧配置、客户端与任务计划。
GET `/api/ui/logs?limit=N` -> `{lines,limit}`，limit限制1..1000，日志脱敏；原前端负责倒序和级别筛选，Rust输出必须保留可解析level与时间。

## 4. 订阅与预览

保留添加、预览、filter、toggle、completed、exclude、correct、metadata、remove、delete与立即扫描接口，具体路径见索引。
AddRSS/Preview/Correct的字段边界按原Pydantic schema实现：名称<=200字符，episode_offset 0..999，override 0..99，订阅排除式<=2000字符，全局<=4000。
这些是Unicode字符约束，不是字节数。协议允许season=0，不能被default逻辑改1。

添加：confirmed=false只返回确认所需信息，不持久订阅、不提交下载。
预览：无配置/资源/任务写入、无外部写操作；允许读远程RSS/TMDB/LLM。
确认：一次性保存完整订阅并安排单源扫描；HTTP retry/双击不得创建两个订阅或两份任务。
暂停/完结：保留订阅元数据，停止新扫描；对已经提交的远程任务不做未授权删除。
恢复：清除completed并启用；不把已入库集数重新下载。
删除：移除追踪但不删除网盘视频；取消尚未提交的任务，已提交任务按已保存计划处理或显式隔离，不能猜测删除。
修改：original_url找到目标，URL冲突409，不得覆盖另一订阅；稳定subscription_id保持不变。

Rust当前实现的管理路由为`POST /api/ui/rss/toggle`、`/completed`、`/exclude`、`/correct`、`/remove`、`/filter`，以及兼容删除入口`DELETE /api/ui/rss`。添加、修正和恢复追踪会触发后台扫描；暂停、完结、删除和过滤规则变更只影响后续周期扫描，不额外启动一次扫描。所有配置修改均经过revision检查和原子替换。

当前尚未实现的部分必须明确视为迁移缺口：元数据重新识别、TMDB/LLM补全、删除时取消未提交任务、稳定subscription_id、Mikan登录/账号订阅与assistant集成，以及完整网页验收。Rust删除订阅只停止后续追踪，不删除网盘文件，也不取消已提交任务；任务隔离/取消语义补齐前，不得宣称与完整旧流程等价。

Rust现已提供`/`、`/login`和`/assets/*`；首页/登录页禁止缓存，登录页和assets在启用鉴权时保持公开，其他网页重定向登录、API返回401。默认从源码树定位网页目录；脱离源码树运行时通过`OPENLIST_ANI_WEB_ROOT`指定现有web目录。该合同测试不等于真实浏览器验收。

预览追加字段建议为`download_plan`，同时保留旧字段；要将现有网页预览/详情区域补成能展示**最终文件名、完整路径、逻辑季、文件季、目录季**。
这是必要可观测性修改，不是重做UI风格。任务列表/详情必须使用冻结计划或实际final_path，不能拿最新配置临时重算。

## 5. 下载与恢复API

保留POST/GET `/api/downloads`、GET `/api/downloads/{task_id}`、retry、archive等。
Task字段：`id/title/download_url/state/anime_name/season/episode/fansub/quality/progress/error_message/retry_count/created_at/updated_at/started_at/completed_at/save_path/final_path`。
保留状态字面值；恢复后task ID、创建时间、远程绑定不变。
成功create必须有实际持久task；policy拒绝/未知结果不发success:true。
retry必须先reconcile。archive隐藏失败历史，不删除其远程视频，也不把未知远程结果当成已清理。
POST `/api/restart`先完成HTTP响应与检查点flush，再退出，由既有systemd Restart策略恢复；测试必须验证是新PID而不是响应后进程挂死。

## 6. 蜜柑、RSS解析与种子入口

Mikan search/groups/rss请求按原字段`keyword/base_url/bangumi_id/subgroup_id`，保留配置base URL和代理行为。
不能硬编码回mikanani.me；当前用户可能使用mikanani.kas.pub。
`/api/parse_rss`是只读解析，不自动下载；limit语义/条目顺序与原合同保持。
`/api/resolve_magnet`与`/api/resolve_torrent`返回`success/message/title/source/file_count/files`。
无可信title时必须null/明确失败，不能把40位hash当作品名称。

上传原协议是**raw request body**，不是multipart：

```text
POST /api/ui/torrent
Content-Type: application/x-bittorrent
X-Filename: encodeURIComponent(file.name)
X-Title: encodeURIComponent(optional title)
Body: File bytes
```

保留这个协议，否则原网页上传会坏。stream验证真实长度，不能信任Content-Length。
解析uploaded bytes后正确转magnet提交；不能将仅本机可访问、需要登录Cookie的HTTP地址交给外部下载器。
保留GET `/api/ui/uploads/{filename}`接口供旧任务兼容；只允许UUIDhex.torrent格式和专用上传目录，仍受原登录校验，不开放任意文件。
旧task payload中已有upload URL恢复时，可在授权路径下直接读取对应本地文件并转换，不能公开其访问权限。

## 7. 合同采样与比较

在合成临时配置/DB、mock外部服务下启动原Python参考版本与Rust，采集所有保留端点的请求/响应。
替换时间、随机nonce/task UUID时必须映射关联一致；不能删除season、episode、path、state或error后比较。
已列明行为变化采用本规格预期，不以错误旧输出为golden。
每个接口至少有成功、非法输入、未登录样本；具有副作用的接口还要重复调用/未知结果/恢复样本。
JSON contract映射清单和真实实现清单必须全覆盖接口索引，缺一项就阻止发布。
