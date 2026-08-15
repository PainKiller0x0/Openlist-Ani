# OpenList / SmartStrm 迅雷令牌同步桥

迅雷的 `refresh_token` 会轮换。OpenList 和 SmartStrm 如果各自刷新同一份令牌，可能互相覆盖，最终表现为其中一边突然提示登录过期。

这个可选部署组件会定期对账 OpenList 与 SmartStrm 的迅雷令牌：

- 默认每 30 秒检查一次，不会每次检查都刷新令牌；
- 以 OpenList 管理 API 读取持久化状态；如果 SmartStrm 先完成刷新，也会把结果同步回 OpenList；
- OpenList 更新时如果再次轮换令牌，会重新读取最终值并同步回 SmartStrm；
- 重载 SmartStrm 前检查 8024/8025 是否有活动连接，播放中会延后到空闲时处理；
- 日志只记录令牌过期时间和不可逆短摘要，不记录令牌本身。

## 使用前提

该桥接器适合以下部署方式：

- OpenList 和 SmartStrm 位于同一台 Linux 主机；
- OpenList 使用本机 `data.db`，并且迅雷存储可以通过管理 API 更新；
- SmartStrm 使用原生迅雷驱动；
- OpenList 管理 API 与 SmartStrm 的配置文件均由 root 管理。

## 安装

把 `xunlei-token-bridge.py` 安装到 `/usr/local/sbin/`，把两个 systemd 文件安装到 `/etc/systemd/system/`，并创建 root-only 的环境文件：

```bash
umask 077
cat >/etc/xunlei-token-bridge.env <<'EOF'
OPENLIST_ADMIN_TOKEN=替换为OpenList管理API令牌
# 以下均可按实际部署覆盖，默认值见脚本
# SMARTSTRM_CONFIG=/opt/smartstrm/config/config.yaml
# OPENLIST_DB=/opt/openlist/data/data.db
# OPENLIST_URL=http://127.0.0.1:5244
# OPENLIST_STORAGE_ID=2
EOF
```

然后启用定时器：

```bash
systemctl daemon-reload
systemctl enable --now xunlei-token-bridge.timer
```

检查：

```bash
systemctl status xunlei-token-bridge.timer
journalctl -u xunlei-token-bridge.service -n 50 --no-pager
```

首次部署前请确认 `OPENLIST_STORAGE_ID` 指向迅雷存储，且 OpenList 管理 API 令牌具备更新存储配置的权限。令牌被迅雷撤销或要求验证码时，仍需人工重新授权一次；桥接器不会绕过此验证。
