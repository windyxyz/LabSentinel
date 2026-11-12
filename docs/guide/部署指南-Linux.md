# LabSentinel Linux 部署指南（T-610）

> **2026-11-11 起不再提供 Docker 部署路线**（维护者决策：Docker 开发终止，产物已
> 移除——原 compose/Dockerfile 的绑定回环、go2rtc 共享配置等问题不再修复；如需
> 容器化请自行封装 systemd 部署步骤）。当前唯一路线：**systemd 原生部署**。
> 两者都只绑回环（T-101/Q-04）：局域网访问须按 1.1.0 的 TLS + 设备配对（T-620）
> 配置后再放开。

## 一、systemd 原生部署

```bash
sudo useradd -r -s /usr/sbin/nologin labsentinel
sudo mkdir -p /opt/labsentinel /var/lib/labsentinel/data
sudo rsync -a server/ /opt/labsentinel/server/
python3.12 -m venv /opt/labsentinel/venv
/opt/labsentinel/venv/bin/pip install --require-hashes --no-deps \
  -r /opt/labsentinel/server/requirements-linux.lock
sudo mkdir -p /etc/labsentinel && sudo tee /etc/labsentinel/config.env >/dev/null <<'EOF'
LS_ADMIN_TOKEN=<占位：用 python3 -c "import secrets;print(secrets.token_urlsafe(24))" 生成>
EOF
sudo cp deploy/systemd/labsentinel.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now labsentinel
systemctl status labsentinel
```

说明：
- unit 已含加固：`NoNewPrivileges` / `PrivateTmp` / `ProtectSystem=strict` / `ProtectHome`，
  仅 `/var/lib/labsentinel` 可写。
- 推理后端默认 `openvino,cpu`（Intel 核显覆盖广）；NVIDIA 机器改 `LS_BACKEND_ORDER=tensorrt,cuda,cpu`。
- 日志：`journalctl -u labsentinel -f`；应用日志在 `/var/lib/labsentinel/data/logs/server.log`。
- 模型放 `/opt/labsentinel/models/`（人体 `yolo11n.onnx` 必选；实验服模型按 Q-08
  网页端确认许可后按需下载）。

## 二、Rust 单二进制（实验性，非容器）

```bash
python scripts/build_rust.py --zip        # 产物：Resources/dist/labsentinel-rust-*.zip
```

单二进制**不含 CPython 运行时**；模型目录走本地路径。当前控制面已与 Python 契约
对齐（105/106 路由），分析数据面（Rust sidecar）按 S7 迁移节奏逐步接管。该产物
仅作 nightly 实验，不进正式 Release（见 `docs/plan/status.md`）。

## 三、验收清单（§13.7）

- [ ] `systemctl status labsentinel` active，`/api/health/ready` 返回 200
- [ ] `tools/e2e_smoke.py --base-url http://127.0.0.1:8000` 通过（文件源→事件）
- [ ] 4 路文件源 5fps 连续 30 分钟 RSS 稳定（`systemd-cgtop` / RSS 日志）
- [ ] 重启服务后数据/令牌保留（`/var/lib/labsentinel` 持久化生效）
