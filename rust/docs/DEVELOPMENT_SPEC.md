# 开发规格与完整实现框架

## 1. 目标与范围

构建一个原生 Linux Rust 服务 `openlist-ani-rs`，替换 `openlist-ani.service` 当前 Python 业务后台。
已有网页原样复用，不新增前端常驻服务，不引入 Node/Python/Chromium 运行依赖。
实际视频下载继续交给 OpenList/Thunder，播放和 STRM 继续交给现有 SmartStrm/Jellyfin；不得把它们并入本项目。

### 必须保留

- 登录、原 Cookie 会话、网页与原 `/api/*` 接口。
- RSS 添加、预览、确认、立即扫描、暂停/恢复、手动完结、删除、单源/全局排除。
- 蜜柑兼容站点搜索、作品/字幕组选择和 RSS 地址构造。
- 普通 RSS 与蜜柑源、magnet、HTTP(S) `.torrent`、直接上传 `.torrent`。
- LLM 元数据解析与批处理；正则模式；TMDB 搜索、按 ID 查询、人工修正及保守校验。
- 文件名标题、作品目录名、文件名季号、目录季号、集数扣除、全局重命名模板。
- OpenList 下载/转存查询、文件查找、冲突处理、移动、改名、失败重试/归档与任务恢复。
- 资源历史、正确的更新集数、用户配置、日志倒序与级别筛选。
- SmartStrm 成功入库触发；失败不重复下载。
- 首页大中小卡片、黄色暂停/绿色完结/蓝色播放恢复/红色删除图标、状态筛选与 localStorage 记忆。
- 排序保持追踪 -> 暂停 -> 完结，各组 TMDB ID 降序；状态操作时卡片位置不立即重排，完整页面刷新才重排。

### 用户批准舍弃

- `assistant/` 的 agent loop、长记忆、dream、skill runner、聊天会话、TUI。
- 上游 Telegram/飞书/微信聊天机器人前端、技能目录及多模型聊天 provider。
- 所有通知机器人和 notification queue；兼容旧配置但不创建这些线程/连接。
- 泛用插件加载和多后端抽象框架；当前只实现真实在用的 OpenList 适配。

注意：`assistant/skill_support/mikan_client.py` 当前被网页使用，不能因路径含 assistant 就一起丢弃。
注意：关闭聊天助手不等于关闭 LLM 元数据解析，两者必须分开。
通知已关闭不等于取消 SmartStrm 触发，这是一条必须保留的入库链路。

## 2. 架构原则

单 Cargo package、单后台进程、单 Tokio runtime（默认 2 worker threads）。
不要建立几十个只转发参数的 trait/registry/crate；仅在 HTTP 外部服务、时钟、数据库与故障注入处设置必要测试 seam。
不要搭建 Redis、向量库、消息队列、数据库服务器、网页渲染浏览器或任何常驻 helper。

```text
浏览器/现有 Nanobot 调用方
       |
现有 Caddy -> Rust API + 原静态网页
                    |
       Subscription / Preview / Naming / TMDB / LLM
                    |
       Scheduler -> Durable Task Worker -> OpenList REST
                    |                       |
         SQLite resources/mementos       Thunder/网盘
                    |
          已验证文件 + 入库事务 -> SmartStrm 触发
```

原路径和域名不变；Rust 只监听 loopback。端口从当前 config/Caddy 读取并校验，不在文档里猜固定端口。

### 目录与职责

```text
rust/
  Cargo.toml / Cargo.lock
  src/
    main.rs                 # 参数解析、初始化、SIGTERM/SIGINT、graceful shutdown
    lib.rs                  # 可集成测试入口
    config.rs               # TOML typed view + 原子保存，保留未知字段/注释
    model.rs                # Subscription/Release/Task/Plan 公共数据类型
    api/{mod,auth,rss,settings,downloads,logs,torrent}.rs
    service/{subscriptions,preview,scheduler,ingestion}.rs
    metadata/{parser,regex_rules,llm,tmdb}.rs
    naming.rs               # 纯函数：完整 DownloadPlan
    storage/{mod,resources,mementos,operations,migration}.rs
    integrations/{http,openlist,mikan,smartstrm,torrent}.rs
    worker/{mod,download,reconcile}.rs
    telemetry.rs            # 脱敏、有限日志尾部、内存/健康指标
  web/                      # 原 HTML/CSS/JS，以 hash 校验复用
  tools/magnet-metadata-helper/  # 必要时原生按需 helper，不常驻，不含 Python
  tests/{contract,rss,metadata,naming,workflow,recovery,migration,security}.rs
  tests/browser/            # Playwright 实际服务测试
  tests/performance/        # 30 分钟稳定性、内存/延迟采样
  scripts/{verify,stage,cutover,rollback,measure-memory}.*
  deploy/openlist-ani-rs.service.in
  reports/                  # 非敏感测试报告；真实数据报告不得推 Git
```

可以适当合并小文件；不得为符合目录树建立空模块。`Cargo.toml.example` 是技术选型建议，不是已验证锁文件。
技术基线：Tokio、Axum、reqwest/rustls、serde、rusqlite/bundled、toml_edit、quick-xml、scraper、regex/fancy-regex、tracing。
HTTP 客户端复用；外部请求不在 SQL 事务中；SQLite 同步操作在专属线程或 bounded spawn_blocking 中执行。
本地密码派生不占 Tokio event-loop；单 DB writer 有限队列，持久写入顺序明确。
所有依赖锁入 Cargo.lock，Linux release 构建必须 `--locked`。禁止未经测量引入大型 provider SDK。

## 3. 领域模型与不可混淆的身份

### Subscription

保留原 TOML 字段 `url/name/anime_name/download_directory_name/episode_offset/filename_season_override/directory_season_override/enabled/completed/tmdb_id/poster_url/season/exclude_patterns`。
另加 Rust 内部稳定 `subscription_id` 与配置 revision；迁移时持久一次，不依赖显示名，不因 RSS URL/标题纠正而变化。
稳定 ID 放在 Rust 辅助表，不强迫旧 Python 配置认识新字段。
RSS URL 不随意重排查询参数、改变路径大小写或丢弃 subgroupid。

### Release / Identity

- 原始 title、source_url、download_url 与来源 item ID（若有）。
- 原始解析 `raw_season/raw_episode` 与最终逻辑 `identity_season/episode`。
- 人工钉选的 tmdb_id、字幕组、清晰度、语言、version。
- 命名覆盖只影响输出计划，不改变已经确认的 TMDB 身份。
- 路径/文件名和身份不能互相反向推导。

### DownloadPlan（同一纯函数用于预览和执行）

`subscription_id/config_revision/identity_season/episode/display_name/tmdb_id/task_name/target_directory/final_filename/rename_format_version`。
用户确认预览时冻结版本；排队任务保存计划。配置变化后尚未提交任务可重新预览/生成计划，已提交任务不得静默搬到新目录。
旧接口没有 revision 参数时，按当前配置计算并返回 additive 字段；现有网页显示最终文件名和完整最终路径。

```text
识别身份：JOJO的奇妙冒险 (2012)，逻辑季 8，集 3（合成兼容案例，不宣称这是当前 TMDB 正确季号）
文件标题：套马的汉子
文件季号：1
作品目录：JOJO的奇妙冒险 (2012)
目录季号：6
期望文件：套马的汉子 - S01E03.mp4
期望目录：/迅雷/videos/番/JOJO的奇妙冒险 (2012)/Season 6
```

这些值必须同时成立；文件标题不能变成 TMDB 搜索词，目录季号不能覆盖逻辑身份。

## 4. RSS 与元数据链路

1. 仅扫描 `enabled && !completed` 的订阅，周期默认沿用 300s。启动/立即扫描互斥或合并，不堆积扫描任务。
2. 有限并发读取 feed，单源失败不影响其余。保留 enclosure/link/source ID 优先级和 feed 容错；设置 body 上限与超时。
3. 全局/单源排除规则在昂贵的模型/TMDB请求前应用；编译失败拒绝保存，不静默忽略。
4. 先批量查已处理 title/来源 ID，避免历史每轮重复解析。标记有效失败分类，不将可恢复失败永久“已处理”。
5. 按配置走 LLM 或 regex；LLM 超时/错误显示结构化原因，禁止凭空生成缺失季集数。
6. 应用 episode_offset，保留 raw_episode；已减过的数不能在验证、重试或恢复时再减。结果 <=0 拒绝并解释。
7. 用钉选 TMDB ID/季约束校验；不因搜索第一项的相似英文拼写而误选海报和作品。
8. 生成同一个 DownloadPlan，再判 strict/优先级/version；最后持久入队。

### LLM 必须兼容

复用当前提示词语义、输出类型、批处理顺序和代理配置；通过 HTTP JSON 直接调用已配置 base_url/model。
保留 OpenAI compatible provider；若旧配置使用 Anthropic 元数据 provider，使用独立小 HTTP adapter，而不是聊天 agent。
运行快照是否使用哪种 provider 需部署前重新确认，不能自动改成更便宜模型。
预算：LLM 请求总超时沿用当前 120s，可并发 1—2；失败不能阻塞登录/UI/任务查询。
处理 fenced JSON、无效 JSON、条目数不匹配和缺字段；输出仅作为元数据，不能执行模型生成的命令/路径。
缓存上限默认 1024 条/TTL 24h，key 包含原始标题、parser/model版本、订阅修正规则 revision。

### TMDB 与历史 bug

- 显式 tmdb_id 优先，不自动被搜索结果改写。
- 不能只取第一个搜索结果；对 RSS 作品标题、别名、动画类型、年份与季进行保守评分；模糊结果要求已有修正或返回待修正，而不是猜测。
- 人工修正作品 ID 时，名称、年份、海报、季详情、预览必须来自同一个最终 ID，并使旧缓存失效。
- 网络 429/超时/缺资料是暂时不可验证，不是“该集不存在”，不写永久 strict_rejections。
- 元数据修正 revision 变化后重新评估旧拒绝；不能让之前错误季总集数永久挡住新集。
- 手动完结仍是唯一自动停止扫描的操作。TMDB `episode_count`、`ended`、当前最大集数不能自动替用户改 completed。
- 野生大魔王 source 13/14/15 + offset 12 -> episode 1/2/3；Clevatess II 第 13 集不得因旧一季 12 集信息被永久挡掉。

### Regex 兼容

原 Python 使用 lookaround、Unicode/CJK、具名组、规则顺序和自定义排除 regex；Rust regex 不直接支持 lookaround。
普通规则用 regex，需要的兼容规则可用有步数/输入长度限制的 fancy-regex或显式解析。不得把所有规则盲目转换后声称等价。
支持旧常用模板与排除式；无法兼容的具体 regex 在迁移预检时列出、拒绝切换，不能悄悄清空。
输入按 Unicode 字符处理，不把 UTF-8 字节偏移用于切分中文；保留字幕组、语言顺序、version 选择。
集数范围与分数按 golden corpus 验证；S00 特别篇的正确规则见显式行为变化。

## 5. 配置与数据库

### TOML

保留全部未知/关闭模块字段和注释，不回写秘密为占位符。typed view 只读当前功能，toml_edit负责局部修改。
写入流程：锁 -> 在副本验证 -> 临时文件 0600 -> fsync -> 同目录 atomic rename -> fsync 父目录 -> 更新内存 revision。
失败不得留下半份 TOML；外部网络校验失败不得先改 live client 后恢复失败。
订阅改名/URL纠正需要和 DB 索引绑定一起完成；跨 TOML/SQLite 不存在天然事务，要持久 config-update intent并可恢复，不是假装两次写是原子。
已关闭助手/notification 配置保留，但 Rust 不实例化对应模块。

### 兼容存储

- 原 `data.db` 的 resources、strict_rejections 与索引保留；禁止 truncate/重建后丢历史。实测resources还有`episode_name`列，不能仅按Python初始CREATE语句重建；`reference/legacy-schema.json`是完整结构基线。
- 原 `task_mementos.db` 的 `task_mementos(task_id,state,updated_at,payload)`、metadata、schema_version=1兼容读写。
- 原 JSON payload 中未知字段原样保留，识别不到的 state/schema/checkpoint放入隔离错误列表，不回到 init重新下载。
- 查询进度优先使用精确订阅绑定及逻辑季；MAX episode是“更新至”，不是完整集数/全集已下载。
- 迁移保留 legacy rows，不确定的资源归属不能模糊批量改写。人工文件名更改不能凭名称把别的剧资源并进来。
- 需要辅助表时加 `rs_schema_migrations`、`rs_subscription_bindings`、`rs_resource_bindings`、`rs_operation_journal`、`rs_trigger_outbox`，不得复制一套 resources历史。旧strict_rejections只有url/title/rejected_at，拒绝revision与类别可放辅助`rs_rejection_context`，不要假设旧表已有reason或JSON字段。
- rs_resource_bindings引用已有 resource ID；ID/原行值不变，歧义条目列报告，不自动认领。
- SQLite foreign_keys/busy_timeout明确启用；采用原 journal模式或有报告的兼容 WAL。两个文件不能假装有跨库事务：以任务状态驱动资源登记重放，用唯一键实现幂等。
- 所有 live 行数是快照，不在程序中硬编码 207/19/3；部署前要重新一致性快照。

## 6. 持久下载状态机

公开 Task state 保持：pending/downloading/downloaded/renaming/renamed/notifying/completed/failed/cancelled。
通知被取消，正常 Rust 流程不进入 notifying；旧 notifying 任务先验证文件/入库，再完成，不发送旧通知。

OpenList workflow checkpoint 保持：

```text
init -> submitted -> download_done -> transfer_done -> file_detected
     -> file_resolved -> moved -> done
```

checkpoint、remote task ID、临时目录、下载文件名、已解析文件名、实际最终路径和计划必须持久化。
临时网盘路径保留 `.oani-download-tmp/<task_uuid>`；清理只允许精确本 task目录，不能清父目录/整季目录。

### 正常完整流程

1. 以 task UUID创建 operation intent和临时路径；所有输入/计划已持久。
2. 通过 OpenList `/api/fs/add_offline_download` 提交，task name在提交前带最终文件名季号/标题，并带可追踪UUID。
3. 查询远程下载/转存列表，使用 remote ID/UUID关联，而不是模糊剧名。
4. 等转存完成/文件可见，递归在本任务临时目录查视频；沿用现有“选最大视频”语义，不自动扩展为整季导入。
5. 处理冲突、移动、改名；两个操作可组合，但必须最终 GET/list验证正确路径、文件名、文件大小 >0。
6. 幂等登记资源，保存实际 final_path和完成检查点。
7. 本地完成；在 outbox记录 SmartStrm触发，仅在入库成功之后触发。
8. 清理本task临时目录；失败可重试清理，不删除已入库文件。

### 外部副作用与未知结果

远程 API没有与 SQLite相同的事务；不得宣称网络重试能提供 exactly-once。
先写 intent -> 调用 -> 校验远程结果 -> 写 committed。读请求可以有界重试；提交、移动、改名不可在超时后盲重试。
提交响应丢失：按 UUID检查下载/转存/本task目录；找到一个则绑定；找到多个则隔离并报告；查不到且无法证明未执行则进入 failed + operation outcome_unknown，禁止自动重提。
移动/改名响应丢失：源/目标和原计划对账；目标已正确则提交检查点；两边都存在/不匹配则隔离，不能删除其中一份猜哪份对。
手动 retry首先对账，不直接 reset init。policy/copyright拒绝为永久失败，不自动改别名绕过。
同一订阅同一逻辑集相同候选的并发扫描/重试必须只能创建一条有效任务；较新version/更优资源按旧优先级规则另行处理，不全局仅用episode去重。

### 不阻塞的调度

bounded channel容量256，download workers默认2，OpenList请求并发4，RSS并发4，metadata并发2。
一个失败/不进展task不独占所有worker；等待poll使用timer，不循环忙等。
下载/转存轮询默认5s；沿用转存15分钟无进展watchdog。
下载也必须有无进展检测，但不能把正常很慢的大文件在固定15分钟总时长后失败；通过配置明确“无进展窗口”，默认30分钟，并保留安全对账信息。
定时扫描、立即扫描、恢复任务各自取消可控；SIGTERM停接新写入、停新扫描、flush检查点，在30s内退出。

## 7. 命名与文件安全

默认 `{anime_name} - S{season:02d}E{episode:02d}`；支持旧已使用占位符 anime_name/season/episode/fansub/quality/languages/version和常用宽度格式。
禁止 eval或动态代码执行；其他 Python格式特性若旧配置使用则预检明确报告，不默默退到默认格式。
filename season override和directory season override的0表示“未设置”；逻辑 season=0是有效特别篇，不能按 false丢掉。
空metadata season/episode与0区分；无可信episode拒绝提交，不默认E01让错误资源进库。
所有组件净化Windows/网盘非法字符，拒绝 NUL、`.`/`..`和根目录逃逸。订阅只改全局根目录下作品文件夹，不能通过详细设置提交任意绝对目录。
扩展名沿用真实视频文件，version>1追加vN；冲突处理最多99次，不能覆盖旧文件。
确认后最终文件名以预览计划为准；提交远程task名不再保留错误的旧逻辑季号。

## 8. Torrent / magnet

- 有可用 dn 的 magnet快速取题，不需要初始化DHT；缺失标题不可编造。
- .torrent v1解析 info dictionary原始字节并正确计算infohash，不能重编码dictionary后hash。
- HTTP种子下载保留30s/10MiB预算；直接上传保留50MiB外部合同，但流式落专用临时文件，不在请求/worker反复复制50MiB。
- 上传种子直接解析本地bytes/file，不回调受登录保护的本机HTTP下载地址，避免401循环。
- 将种子转换到OpenList支持的正确磁力URI，不把torrent文档当视频下载。
- v2/hybrid不能被当v1乱算hash：兼容实际旧实现支持的格式；无法处理时明确422/400错误，不是假成功。
- 无dn magnet的元数据补全必须保留；可用按需启动的原生helper封装现有C++ libtorrent，而不是Pythonhelper。
- 主业务100% Rust、无Pythonruntime；底层SQLite/TLS/native libtorrent不要求全部源码纯Rust。
- helper只取元数据、禁下载视频pieces，超时、取消、SIGTERM时必须杀完整子进程组并清理本请求临时文件。
- helper并发1、默认30s（上限120s）、进程资源有界；不得成为后台daemon，不为了启动它改公网防火墙。
- 验收包括本地metadata seed实际协议测试，验证视频piece请求/视频payload为0；不能只mock helper返回值。

## 9. 资源、安全与可观测性

复用TLS/HTTP客户端与有限连接池；每个body、解析缓存、TMDB缓存、日志环、任务内存列表有上限。
任务历史从SQLite分页查，不永久把所有完成任务留在内存。不得每个任务创建runtime/threadpool/httpclient。
参考上限：RSS4MiB/单源1000项，任务内存仅活跃页，日志最近1000行/每行8KiB，TMDB256/128条TTL1h。
超过新上限须明确错误/截断标记，不能把多余项静默当处理成功；预检记录现有源是否超限。

Cookie兼容原PBKDF2/HMAC/stateless格式，保留auth.env；production无有效auth配置拒绝监听，而不是无鉴权开放。
只在loopback或可信Caddy代理来源接受X-Forwarded-Proto，GET设置不得返回secret；日志脱敏key/token/password/query/cookie。
保留原401/303合同；新安全规则允许额外校验危险跨站写入，但测试旧网页正常操作不受阻。
SSRF治理要允许已配置loopback OpenList/LLM/上传流，但普通RSS/种子地址不能借重定向访问cloud metadata或凭据文件。
UI与日志不直接插入未转义的RSS标题/模型内容；不得通过用户名、RSS文件名、路径参数读取任意文件。
提供loopback-only health/readiness：区分API在线、DB可用、扫描是否启用、外部依赖暂时错误。上游暂时不可用不能导致服务退出重启风暴。
日志记录 task/subscription/operation ID、phase、error class、retry/backoff，记录真实开始/完成，不刷屏打印每次poll。

## 10. 显式行为变化（不要机械复刻旧bug）

1. S00特别篇保持S00；空episode不默认为1；override=0仍表示未设置。
2. 所有完成必须先验证实际文件，DB记录/日志不能替代远程文件验证。
3. SmartStrm触发移到最终改名入库之后；触发失败不使视频任务重新下载。
4. mutating请求超时走对账，不盲重试；未知检查点不回到init。
5. 上传种子本地解析，不访问需要Cookie的内部文件URL。
6. 新提交任务名与最终文件名覆盖一致，同时保存UUID；避免旧提交名与预览不同。
7. 新任务保存冻结计划，已提交任务不受配置后改影响。
8. 缓存与拒绝记录绑定配置revision；暂时TMDB失败不永久拒绝。

除上述安全/正确性修正、用户批准去除的模块和自动化验收必要字段外，不顺便重做UI、改提供商、改路径、改密码、改下载策略。

## 11. 性能验收目标（必须实测）

- Linux release，原生systemd、1个主进程，常态无helper。
- 同一VPS、同一脱敏数据/19源形状、相同请求负载，分别测Python与Rust，不和凌晨Bing峰值混比。
- 每阶段至少30秒采样；主稳定性测试至少30分钟，覆盖至少6次扫描及浏览器轮询。
- 常态目标PSS <=40MiB；warm cgroup anon+kernel报告，memory.current含cache另列。超过目标必须分析，不隐藏cache或强制drop_caches。
- 常规RSS/任务负载目标主进程PSS peak <=80MiB；50MiB上传与native helper单独测峰值，不把异常负载冒充常态。
- 30分钟相同负载后PSS净增 <=5MiB作为回归阈值，不将allocator retained memory单凭一次快照称为泄漏。
- 无外部请求的state/logs/login-session接口本地p95 <=100ms；metadata挂120s时这些接口仍可用。
- 2分钟并发轮询不产生busy loop；空闲CPU平均 <=1%单核（独立进程采样，非整机CPU）。
- 系统memory pressure、Swap、其他任务变化必须附报告，不能用swap-out伪装内存优化。

## 12. 完成定义

所有保留接口有真实实现；所有P0/P1用例有自动化断言及报告；旧数据/会话/网页通过；所有状态检查点重启恢复通过；未知副作用不重复提交；性能达标或在部署前明确阻塞。
交付Linux二进制、锁文件、源码、systemd模板、安装/回滚/测试脚本、fixtures、截图、内存报告和部署核验报告。
开发代理负责验证；若外部平台必须账户交互才能验证，列明唯一不可自动验证项，不把整个验收推给用户。

## 13. 技术来源（实现前再核对）

- [Axum 0.8](https://docs.rs/axum/0.8.6/axum/)：Tokio/HTTP API架构基线。
- [rusqlite 0.37](https://docs.rs/rusqlite/0.37.0/rusqlite/)：本地SQLite与备份接口。
- [toml_edit](https://docs.rs/toml_edit/latest/toml_edit/)：保留TOML文档结构。
- [regex](https://docs.rs/regex/latest/regex/) 与 [fancy-regex](https://docs.rs/fancy-regex/latest/fancy_regex/)：语法能力与复杂匹配限制。
- [libtorrent Core](https://www.libtorrent.org/reference-Core.html)：metadata-only的pieces限制必须按实际选用版本验证，不能只在metadata到达后调用pause。

依赖文档是能力参考，不能替代本项目的编译和兼容测试。
