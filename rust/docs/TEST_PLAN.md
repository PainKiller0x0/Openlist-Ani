# 自动化测试与验收计划

开发代理负责落实、运行与解释全部测试。用户不承担常规手动回归。
`fixtures/acceptance-cases.json`是带ID、前置条件、动作、预期的机器可读验收矩阵。
`fixtures/golden-naming.json`是独立计算出的命名golden，不允许由被测naming函数生成expected。

## 1. 测试层与禁止事项

| 层 | 实际被测对象 | 可以mock什么 | 不能替代什么 |
| --- | --- | --- | --- |
| 单元 | 命名/偏移/身份/解析/排序/格式校验 | 时钟/输入 | HTTP与持久恢复 |
| Adapter | 真实Rust HTTP client + 本地stub server | OpenList/TMDB/LLM/Mikan响应 | reqwest序列化、鉴权、重试规则 |
| 合同 | 实际监听的Rust服务+SQLite+原网页客户端协议 | 外部平台 | Axum验证/返回状态/Cookie |
| 恢复 | 子进程Rust + SQLite + 动作日志 | 外部平台可控响应 | SIGTERM/SIGKILL/重启/DB重读 |
| 浏览器 | Playwright + 实际Rust HTTP +真实SQLite+原静态UI | 外部平台 | 页面数据流，不允许page.route把/api全mock掉 |
| Helper | 原生helper + 本地metadata seed | 公网tracker | 真正metadata握手与无pieces下载 |
| 预发布 | Linux、同身份、生产副本数据、现有文件查询 | 外部写操作默认禁用 | ABI/权限/Caddy/路径与状态 |
| 生产canary | 经授权真实OpenList受控测试目录 | 不mock实际文件完成 | 真实网盘命名/可见性/联动 |

禁止只用UI截图证明下载成功；必须查询最终文件实际存在且有正确名称/大小。
禁止mock业务service自身返回固定success；mock只放在外部网络seam。
禁止所有测试ignored、只实现通过数统计或把case JSON校验当cargo测试通过。

## 2. Harness要求

- 临时配置/SQLite/上传目录使用每次运行唯一目录，固定synthetic auth，不用真实config/env/token。
- 测试启动二进制监听127.0.0.1随机空闲端口；先等readiness，不sleep猜启动完成。
- Stub记录全部请求method/path/body/headers/顺序；测试必须检查每种副作用调用次数及UUID关联。
- 可注入单调时钟/seeded随机；超时/backoff用虚拟时间，避免每例等15分钟。
- DB校验包括重新打开、foreign_key_check、integrity_check、行数、ID、payload、实际文件列表。
- 崩溃测试在每个关键“intent已写/远程已执行/commit未写”点杀进程，不能只调用一个恢复函数代替。
- Cleanup仅本run目录/测试对象；finally/signal后检查无worker/helper/文件残留。
- 每个case ID对应真实test名称或manifest映射；测试报告需逐ID列executed/pass/fail/skip和原因。
- 外部HTTP不做无条件公网请求。确定性CI全部本地，真实生产canary需授权且不混入普通cargo test。

## 3. 必须覆盖的历史回归

### JOJO三套名称/季号

合成身份：逻辑季8集3；显示JOJO (2012)；文件标题套马的汉子；fileSeason1；dirSeason6。
预览=`套马的汉子 - S01E03.mp4`，最终路径=`/迅雷/videos/番/JOJO的奇妙冒险 (2012)/Season 6/套马的汉子 - S01E03.mp4`。
API/提交前task_name/实际rename/重启后的final_path一致；TMDB不搜索“套马的汉子”。
配置改成另一个名字后，旧已提交task仍完成到旧冻结计划，新任务使用新revision。
已有3条资源仍只展示更新至3，不能把history/title别名当5集；只有MAX=3并不证明1/2/3全在。

### 野生大魔王累计集数

RSS 13/14/15，offset12，逻辑季2 -> E01/E02/E03；恢复和retry不再次减12。
13/14已有，15新 -> 只新建一个E03任务；完成后progress=3且实际Season2文件存在。

### Clevatess II第13集

TMDB旧cached S01共12集；订阅已钉选正确S02；RSS有E13。
不能依据旧season cache写永久rejection。新TMDB S02存在13则正常；TMDB暂不可用进入retry/deferred，不自动完结。
修正TMDB/season后原strict_rejection可重新评估，两个重复poll仍只能提交一个任务。

### TMDB错误作品与海报

蜜柑RSS bangumiId4064/subgroup583，合成搜索响应第一项ID336654但不匹配，正确钉选283938。
期望ID283938、对应标题/年份/海报来自同一个fixture对象；不能取第一个搜索项海报、却保存另一个ID。
这里ID与title是回归输入，不依赖公网TMDB今天的内容；fixtures不会作为生产硬编码例外。

### 网盘政策拒绝

OpenList HTTP200但JSON code!=200且message为“应版权方要求，无法添加”。
任务failed，error class=policy_denied，零资源写入、零SmartStrm、零自动alias重试。
错误信息要原样可见（脱敏后），不能只显示“未知错误”。

## 4. 浏览器验收

Playwright直接请求真实Rust API/SQLite，桌面1366x900，手机390x844，另测360px。
必须自动完成：登录/退出、首页卡片、四状态筛选/刷新后记忆、大小切换、按钮颜色/可点击区域、默认配置/详细设置、预览/确认、人工ID修正、改名、暂停/完结/恢复/删除、立即扫描、下载列表/重试/归档、日志倒序与level过滤。
每操作同时断言API/数据库结果，禁止“按钮能点”就通过。
原localStorage键：`op-ani.subscription-card-size`、`op-ani.subscription-filter`，不得换key丢偏好。
卡片状态切换当前页面不重排，刷新后追踪->暂停->完结，各组TMDB ID降序。
默认配置字段不能全部藏进展开；详细设置中的逻辑季/file季/dir季及最终文件计划看得清楚。
红删除需要确认；操作区域不触发外层卡片跳转；大卡片按钮要随布局适配，不缩成小卡片尺寸。
无pageerror、未处理Promise rejection、接口404/500、横向溢出；截图需存档供代理自己检查，不让用户补截屏。
退出验证浏览器不再访问私有API；不能错误断言旧stateless token全球撤销。

## 5. 数据、回滚、故障

- 旧SQL schemas、task schema1和所有workflow checkpoints导入、继续执行、回写旧格式均测。
- 完成/失败/归档/取消/未知state分别处理；未知state拒绝开scanner而不是重新下载。
- TOML未知section/凭据/注释原样保留（fixture只有synthetic secret）；部分字段修改不能清空其它字段。
- 中途断电在每处配置写入/跨DB登记后重启，最终保持旧完整配置或新完整配置，不半写。
- 数据库损坏是明确拒绝启动/保持备份，不自动创建空DB；文件缺失同样不悄悄初始化生产。
- 迁移可重入，重复导入不加重复资源/任务；schema升级失败交易回滚。
- 备份后的新记录、新task、用户新改配置在rollback中仍存在，不恢复陈旧快照覆盖它们。
- 重复rollback为no-op；重新cutover需要新rollback point。
- rollback时outcome_unknown仍保持failed可见，原Python启动不得自动重提；必须实际跑旧恢复逻辑验证。
- 已提交的远程操作必须对账；未知move、两个目标文件都存在、源/目标都缺失应隔离，不自动删文件猜结果。

## 6. 性能/内存测试

同一Linux host、release构建、同形状数据和相同负载完成A/B；每段至少30秒，稳定性至少30分钟。
报告PSS/RSS/SwapPss、cgroup memory.current与anon/file/kernel、CPU、线程/FD数量、p50/p95延迟、吞吐/真实请求数。
不能仅比较VSZ；不能把RSS/PSS/cgroup混在一列；不能强制drop_caches或swap-out再截图证明省内存。

阶段：
1. 启动/登录/UI轮询warmup后空闲30s。
2. 19源（4active）的feed与207resource扫描30s，带本地stub元数据。
3. 两个活跃download任务轮询30s，元数据请求挂起，UI仍响应。
4. 50MiB上传与helper metadata，单独记录峰值/回收。
5. 相同RSS/UI负载30分钟，至少6轮scan，内存/FD不持续增长，任务终态从内存移出。

发布gate：常态PSS<=40MiB；常规峰值<=80MiB；30分钟净增长<=5MiB；无外部网络接口本地p95<=100ms；空闲CPU<=1%单核。
这些阈值是验收目标，不是本包已完成测试的结果。超过阈值要阻塞发布并分析，不能擅自改阈值为当前结果。
helper与大上传峰值要报告并在服务配置中保留余量，不硬限到40MiB导致OOM。

## 7. CI与一键验证

接手实现需提供：`scripts/verify.ps1`用于Windows本地、`scripts/verify.sh`用于Linux/CI。
默认离线mock网络、自动创建数据库/服务/浏览器，失败立即非零退出；不存在实现的阶段不得默认success。

```text
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --release --locked
node tests/browser/run.mjs
./scripts/verify-contract-coverage
./scripts/verify-recovery
./scripts/verify-helper
./scripts/measure-memory --duration 1800
```

Linux GitHub Actions包含unit/contract/migration/recovery/security/Playwright/native helper构建；性能阈值在指定host运行，CI只检查采样结构与趋势，不能用共享runner硬套VPS绝对性能。
测试专用依赖可包含Playwright/Node/Python参考oracle；生产Rust二进制不能依赖这些测试运行时。
参考Python若无法在CI干净安装，应使用已审查golden与schema fixture，并将缺失的差分测试列阻塞项；不要在VPS生产venv上跑破坏性测试。

## 8. 报告模板

每份报告记录git commit/source hash、Rust版本/lock hash、OS/架构、测试开始结束、fixture版本、命令与退出码。
`reports/acceptance.json`逐case ID列status/test_name/evidence_path/error；`reports/api-coverage.json`逐endpoint列HTTP用例；`reports/memory.json`保留原始时序；`reports/recovery.json`保留fault point/remote mutation counts/row checks。
markdown总结必须区分已实现、已执行、mock验证、真实外部验证、未验证；不得只写“全部通过”。
真正完成后给用户交付自动生成的结果摘要，不让用户按开发书再走一遍手工流程。
