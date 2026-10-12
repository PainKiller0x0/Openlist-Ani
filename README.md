# OpenList-Ani

OpenList-Ani（op-ani）是一个面向个人媒体库的 RSS 追番与下载任务管理工具。它负责发现新资源、识别番剧与集数、预览命名和目录，并把确认后的下载任务交给 OpenList。

**当前默认分支 `master` 是 Rust 实现。** 原有 Python 版本保留在 [`python` 分支](https://github.com/PainKiller0x0/Openlist-Ani/tree/python)，作为历史实现和兼容参考。

## 工作流程

```text
Mikan / RSS
    ↓
op-ani：解析、识别、过滤、预览最终文件名和目录
    ↓
OpenList：创建离线下载任务
    ↓
SmartStrm（可选）：生成 STRM 并触发媒体库刷新
    ↓
Jellyfin / VidHub 等客户端
```

OpenList 是下载和网盘存储入口；SmartStrm、Jellyfin 和播放器不是运行 op-ani 的硬依赖。

## 当前能力

- Rust/Axum 后端、登录会话、RSS 追踪页面、设置、日志和下载任务管理。
- Mikan 搜索作品、字幕组并构建 RSS；也可以直接添加 RSS。
- 预览 RSS 识别结果、排除规则、文件名和完整下载目录，再决定是否开始追踪。
- 通过 TMDB 校验作品身份和季集信息；支持配置 OpenAI/Anthropic 兼容服务辅助识别。
- 独立设置文件名标题、文件名季号、归档季目录和集数偏移，适配特殊作品命名。
- 向 OpenList 提交下载任务，持久化任务进度，支持受控重试和失败任务归档。
- 下载完成后按规则重命名、移动；可选通知 SmartStrm。
- 兼容现有 TOML 配置和 SQLite 业务表；Rust 下载流程另用持久化状态表记录任务，不重建旧表或搬动既有数据。

## 仓库结构

- `rust/`：当前 Rust 服务、测试、部署脚本和迁移文档。
- `src/openlist_ani/adapters/inbound/http/web/`：Rust 服务使用的网页资源。Rust 从文件系统读取该目录；离开源码目录部署时使用 `OPENLIST_ANI_WEB_ROOT` 指向它。
- `python` 分支：原 Python 版本，保留原有 README 和代码历史。
- `rust/docs/IMPLEMENTATION_STATUS.md`：已实现能力和仍未迁移的功能。
- `rust/docs/API_CONTRACT.md`：HTTP/API 行为和兼容性说明。
- `rust/docs/DEPLOYMENT_RUNBOOK.md`：隔离验证、部署、验收和回滚流程。
- `rust/docs/LIVE_RELEASE.md`：已完成的线上切换记录和验证证据。

## 本地开发与测试

需要 Rust 1.85。所有 Rust 命令从 `rust/` 目录执行：

```sh
cd rust
cargo +1.85 fmt --all -- --check
cargo +1.85 test --locked
cargo +1.85 clippy --locked --all-targets -- -D warnings
cargo +1.85 build --locked --release
```

这些命令验证 Rust 代码，不会启动真实下载或连接生产网盘。运行服务需要有效的配置文件、兼容的 SQLite 数据库，以及通过环境变量提供的认证信息。请使用隔离的测试配置和数据库；不要把生产凭据或数据库提交到仓库。

完整的服务启动、配置和生产操作要求见 [`rust/README.md`](rust/README.md) 与部署手册。生产服务应只绑定本机回环地址，并通过受控反向代理访问。

## 迁移边界

Rust 版本已承接主要追番、RSS、识别和下载流程，但**不宣称与 Python 旧版的全部功能完全等价**。例如，磁力链接的 DHT 元数据解析、Mikan 账号登录/订阅、聊天机器人和助手集成，以及部分旧接口仍有迁移缺口。完整清单和当前验证状态见 [`实现状态`](rust/IMPLEMENTATION_STATUS.md)。

## 安全说明

- 不要提交 `config.toml`、认证环境文件、API Key、机器人令牌、生产数据库、日志或上传的种子文件。
- 使用 `config.toml.example` 作为配置结构参考；上线配置应放在仓库之外并限制文件权限。
- 不要把服务直接暴露到公网；通过回环绑定、反向代理和访问控制提供网页入口。
- 部署、重启或回滚生产服务前，请遵循部署手册并保留可验证的回滚点。

## 许可证与上游

本项目基于 OpenList-Ani 相关上游代码持续维护。请查看仓库中的许可证文件及 [`python` 分支](https://github.com/PainKiller0x0/Openlist-Ani/tree/python)中的原始项目说明，了解代码来源与适用条款。
