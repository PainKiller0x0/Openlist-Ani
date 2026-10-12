# 部署、回滚与生产验收

本包不授权立即部署。实施代理必须先完成本地/CI测试，经用户明确批准后执行生产动作。
生产参考：`ssh -p 2222 root@150.158.121.88`；本机PowerShell，不能把bash语法放到本地执行。
复杂远程脚本写文件、LF归一、scp后执行，避免嵌套引号和本地`$var`插值。

## 1. 只读预检

- 核对当前service、WorkingDirectory、ExecStart、运行用户、PID/cgroup、Caddy上游与真实端口。
- 查看Git仓库分支/dirty状态；线上`/opt/openlist-ani`目前无Git，不要在那里执行reset/clean。
- 对用户仓库、线上代码和本包快照diff，保留所有新增修复。快照日期不能替代当前git/source hash。
- 私有配置在server内部解析，只输出字段存在/功能开关/计数；禁止输出auth.env、token、密码hash或完整config。
- 列每个resources/task state/checkpoint计数，SQLite integrity_check/foreign_key_check；识别不能迁移的task和未知格式。
- 检查root/作品目录/任务临时目录约束、外部域名/代理、本机已有端口；**必须实际检查空闲端口**。
- 列出现有外部调用方保留接口：Caddy网页、Nanobot tools、SmartStrm触发、OpenList种子任务。
- 检查定时器/cron/ops-guard自愈逻辑，不让它们在切换时自动重启旧扫描器。

## 2. 命名回滚点

创建 `/opt/openlist-ani/backups/pre-rust-<UTC timestamp>/`，目录0700、含秘密文件0600。
服务运行中用SQLite backup API取一致快照，不裸cp WAL中的.db。
备份：config.toml、auth.env、两个DB及journal信息、uploads/legacy task JSON、运行Python源码、systemd单位及drop-in、相关Caddy片段、源码/配置hash和service enable状态。
秘密不能进入开发包/Git/reports公开目录。
迁移前config/database需要锁或冻结写入窗口；停服务后的最终快照必须覆盖初始快照之后的新改动。
manifest记录版本/路径/计数/hash/owner/mode，不记录secret值；逐项检查备份可读取与DB可打开。
不删除原venv/source/static/数据库和原unit，直到稳定验收结束。

## 3. 预发布隔离

安装Linux x86_64 release二进制到独立release目录，校验SHA256与ABI（Linux不是Windows exe）。
新service名 `openlist-ani-rs-stage.service`，仅loopback实际空闲端口，单独staging目录与数据库副本。
必须以与生产相同身份测试访问权限；不要以root test通过后改成另一个用户却不测试。
本次优先复用已有service身份，不顺便全盘重分配数据owner；以后要非root hardening另行评估。
staging `scan_enabled=false`，外部写deny-by-default；复制的未完成task只读展示，不自动恢复远程操作。
真实API只读：OpenList健康/目录列表、TMDB必要查询可执行，不能擅自提交下载/移动/改名/删除。
预发布的所有模拟执行使用本地stub；不改生产config或数据库。

核验原cookie兼容（可用server内部临时synthetic材料，不输出真实cookie），207等当前实际行数/ID/字段一致，完整UI访问、静态资产、直接刷新/hash route。
执行已授权的真实canary：专属目录、专属小型合法资源、专属task UUID；开发代理自动查询最终文件名/路径/大小并触发SmartStrm，自动清理仅自己测试对象。
没有安全测试资源或真实下载权限时，明确列为不能验证的阻塞，不用历史文件冒充新下载验证成功。

## 4. 切换gate

必须全部满足：所有P0/P1用例执行通过；全API覆盖；Linux二进制可启动；schema/mementos兼容；自动浏览器通过；metadata helper通过；内存目标通过；rollback演练通过；真实canary通过或用户明确接受具体残余风险。
系统单元和脚本不得包含未渲染的`__PLACEHOLDER__`；测试命令/程序不存在不得报告OK。

执行顺序：
1. 获取迁移锁/标记maintenance，阻止新写入；暂停自愈重启旧程序（只影响本服务）。
2. 停原Python，让其flush；确认PID/cgroup退出，没有old worker/helper。
3. 取最终SQLite/config快照，迁移helper只处理必要的Rust辅助表；记录逐行校验结果。
4. 保持旧资源表/task schema1可读写；不复制成另一条长期数据分支。
5. 替换服务ExecStart为Rust、保留同端口/原环境文件和工作目录。先scanner disabled完成只读readiness。
6. 逐一恢复持久task，所有未确定的远程副作用先reconcile；不盲目重新提交。
7. 开启一个Rust scanner，确认旧scanner没有运行；恢复该服务自愈/定时检查。
8. Caddy尽量不改，仅保持原上游端口；若必须改要备份、validate、reload，不能全站stop。
9. 按API/浏览器/文件/内存自动检查。失败自动进入保持最新数据的rollback，不重新启动两套scanner。

## 5. systemd要点

原unit目前运行Python，Rust正式替换后service名仍为`openlist-ani.service`，避免ops-guard识别变化。
需要graceful SIGTERM、TimeoutStopSec=30、Restart=on-failure、RestartSec=3、UMask=0077、loopback监听、TOKIO_WORKER_THREADS=2。
MemoryHigh/Max根据实测主进程+helper+大上传峰值设余量，例如目标期先参考128MiB/256MiB，不能拿40MiB idle目标直接MemoryMax=40M。
这些是参考值，必须由压力测试决定；若helper需要更高单独受限cgroup也要自动化验证。
ProtectSystem/ReadWritePaths/用户权限须实际运行同环境测试，不凭模板推断有权限。
公网端口不新增，Caddy域名与证书不改，云防火墙不动。

## 6. 保持最新数据的回滚

切换后DB/config/网盘会继续变化，**不能把pre-rust快照直接覆盖回来**。
取得迁移锁，停新scanner/写入，flush/停Rust；对所有有远程副作用intent先只读对账。
resources保留最新行；task_mementos schema1保留最新检查点；TOML保留用户切换后修改、未改动秘密。
Rust-only辅助表旧Python可忽略，不能随意drop；outcome_unknown导出为旧格式failed，并验证旧恢复逻辑不会自动重提。
如果存在无法安全表示的checkpoint/config，不恢复旧scanner，进入明确maintenance并报告；不得用回滚“成功”掩盖重复下载风险。
恢复原unit与必要Caddy片段/环境引用；启动旧Python，首先disabled扫描验证数据可读，再确认恢复扫描。
rollback完成标记持久化；重复运行no-op，不能覆盖回滚后在Python新增的数据。
rollback脚本在合成环境实际测试“Rust写新数据->rollback->Python读写->再次rollback”，证明无数据丢失。

## 7. 上线后自动验收与观察

- service active、NRestarts=0、ExecStart/PID对应Rust、loopback端口正确。
- Python旧进程和native helper空闲时不存在；其它服务（OpenList/SmartStrm/LOF/Nanobot）未被改停。
- 公网真实浏览器登录/列表/预览正常，API无误404/500；原cookie或重新登录流程有效。
- 配置/资源/task行数和hash按最终快照校验，异动必须能解释为实际执行。
- 真正完成的任务在OpenList最终路径可见，最终文件符合冻结预览；不能只查DB或日志。
- 至少6轮扫描+30分钟稳定性内存记录，helper退出、FD回收、无重复提交。
- 24小时观察由自动巡检输出异常摘要，不默认发起新的ChatGPT调度；若需要新定时推送先取得用户授权。
- 向用户报告测试、部署版本、内存A/B、备份点、回滚入口与未验证风险，不让用户“自己点点看”。

## 8. Git交付

仅在用户明确授权后commit/push到用户仓库。先扫描secrets与大文件，不提交auth.env/config真实值/DB/uploads/logs/reports私有数据。
保留原Python参考实现和必要license直到用户批准清理；可以移除未用模块的Rust实现范围，不代表允许删旧仓库历史。
CI必须绿；push后实际检查Actions状态，失败由实施代理修复，不说“推完了应该没问题”。
