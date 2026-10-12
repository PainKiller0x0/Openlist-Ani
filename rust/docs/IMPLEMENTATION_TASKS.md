# 给开发代理的可执行工作拆分

这是完整交付的执行顺序，不是可自行删功能的一期/二期。最终所有工作块都必须完成。
允许分派子代理，但写入范围必须互不冲突；由主代理负责集成、真实回测和最终部署核验。
不要把最关键状态机写给另一个代理后只等结果，也不要把每个小函数拆成独立crate。

## A. 固定兼容合同

输入：本包源码、35个旧HTTP路由、26个Pydantic模型、legacy SQL、用户仓库及当前线上diff。
产出：真实可编译Cargo package、AppState、error/JSON规范、auth、静态UI、测试harness、API覆盖manifest。
禁止：空handler一律返回success、mock业务service、未经验证删旧字段、先部署再补测试。
完成：所有HTTP用例都有有意义的成功/输入错误/鉴权断言，旧Cookie样本与原UI登录通过。

## B. 数据与纯规则

负责：config.rs/model.rs/naming.rs/storage与config revision/冻结计划。
产出：旧SQLite兼容读写、旧memento导入/回写、未知字段保留、配置写入恢复、稳定订阅绑定、独立命名golden测试。
完成：NAME/EP/DATA/REC-UNKNOWN全部通过；两次迁移无重复；resources.episode_name不丢；root/目录/文件标题/三种季号不混淆。
无需为了测试拆泛用repository框架；临时SQLite足够。

## C. 外部网络适配

负责：共享HTTP client、OpenList/Mikan/TMDB/LLM/SmartStrm adapter、RSS读取。
产出：真实HTTP序列化/解码、bounded body、proxy、错误分类、有限重试、缓存revision、日志脱敏。
完成：RSS/META与每个实际OpenList端点有stub请求日志断言；POST不默认共用GET重试。
Rust调用模型直接HTTP，不依赖OpenAI/Anthropic大型SDK；保留已有模型和功能。

## D. 状态机与故障恢复（主代理必须集成审查）

负责：worker/download/reconcile、persistent intents、active queue、resources登记、SmartStrm outbox。
函数/数据合同建议：

```text
build_plan(subscription, validated_release, config_revision) -> DownloadPlan | ValidationError
enqueue(plan, source_identity) -> existing_or_new_task_id
advance(task_id) -> progressed | waiting(deadline) | failed(class,outcome)
reconcile(operation_id) -> committed(evidence) | definitely_not_executed | ambiguous
commit_resource(task_id, verified_final_file) -> idempotent_resource_id
flush_and_shutdown(deadline) -> durable_report
```

`definitely_not_executed`必须有可审查证据，不是“暂时列表里没找到”；ambiguous不能自动重发。
正常流程保持public state和legacy checkpoint，额外operation状态只在辅助表里记录。
完成：FLOW与全部REC检查点的真实子进程kill/restart测试通过；每一步副作用调用次数正确。

## E. 种子与metadata helper

负责：torrent parser、raw上传、magnet dn、按需native helper。
完成：正确infohash与文件列表、旧上传协议、50MiB预算/清理、真实本地metadata seed且非零payload不被下载、timeout后无子孙进程。
禁止：保留Pythondaemon、用只返回固定JSON的helper骗过测试、无dn磁力链悄悄取消支持。

## F. 网页完整自动回归

复用原web目录，只有显示最终计划、必要数据/错误处理兼容的小改动。
原有页面/控件全部连接真实Rust API+SQLite，不把/api路由整体拦截成mock。
完成：UI系列和所有endpoint实际浏览器流通过；桌面/手机截图由开发代理查看；不再让用户补测按钮与命名预览。

## G. 性能、部署与回滚

Linux release构建、source/lock hash、一键验证、CI、systemd模板、backup/stage/cutover/rollback脚本。
完成：PERF/DEPLOY/DATA rollback实际测试；相同负载A/B、30分钟soak、生产只读预发布；经授权真实canary。
禁止：未跑的验证步骤输出OK，示例路径/模板未渲染就部署，遇到Actions失败留给用户修。

## H. 交付检查表

每个case ID在reports/acceptance.json有真实测试名和证据；每个接口在reports/api-coverage.json有实际HTTP测试。
报告中未验证项必须单独列出，不能与pass混算。真实外部依赖不可用时，不谎称完全完成。
全部通过后交付可运行Linux版本、README、环境变量说明、升级与回滚操作、内存变化。
未经授权不push main、不删旧Python、不过度硬化不相关服务。

## 必须避免的实现捷径

- `todo!()` / `unimplemented!()` / 固定空数组响应属于未实现，不是可上线功能。
- 在test里写一个假的naming函数然后测试它，不算测试产品naming。
- 以DB有resource行替代OpenList实际文件校验，不算下载验收。
- 只有MAX(episode)=3就报告三集完整可播放，不算媒体验收。
- 程序异常时自动删DB重建、未知checkpoint自动init，属于数据损坏风险。
- 新旧两套程序都运行然后比较线上结果，会重复提交，禁止。
- 源码快照和fixture只帮助建立合同；最终必须执行真实实现与恢复测试。
