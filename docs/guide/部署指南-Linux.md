# LabSentinel Linux 部署指南（Rust 单二进制）

> **2026-11-11 起不再提供 Docker 部署路线**（维护者决策，产物已移除）。
> 2026-11-14（ADR-0011 过渡期结束）起 Python 运行时退役，唯一路线：
> **Rust 单二进制 + systemd 原生部署**。
> 默认只绑回环；局域网访问须设 `LS_HOST=0.0.0.0` + `LS_ADMIN_TOKEN` 后再放开。

## 一、构建产物

```bash
# 前端
cd web && npm ci && npm run build && cd ..

# Rust 服务端（在目标机或同架构构建机）
cd engine-rs && cargo build --release -p ls-server && cd ..
# 产物：engine-rs/target/release/labsentinel-server

# 可选：完整发行包（模型/ffmpeg/onnxruntime 资产按哈希锁准备）
python scripts/prepare_rust_assets.py
python scripts/build_rust.py --strict --zip
```

## 二、systemd 原生部署

```bash
sudo useradd -r -s /usr/sbin/nologin labsentinel
sudo install -d -o labsentinel -g labsentinel /opt/labsentinel /var/lib/labsentinel/data
sudo install -m 755 engine-rs/target/release/labsentinel-server /opt/labsentinel/
sudo cp -r web/dist /opt/labsentinel/web
sudo install -d /opt/labsentinel/models
sudo cp m0-spike/yolo11n.onnx /opt/labsentinel/models/   # PPE 模型存在则一并复制

sudo mkdir -p /etc/labsentinel && sudo tee /etc/labsentinel/config.env >/dev/null <<'EOF'
LS_ADMIN_TOKEN=<占位：用 python3 -c "import secrets;print(secrets.token_urlsafe(24))" 生成>
# LS_HOST=0.0.0.0
EOF
sudo cp deploy/systemd/labsentinel.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now labsentinel
systemctl status labsentinel
```

说明：
- unit 已含加固：`NoNewPrivileges` / `PrivateTmp` / `ProtectSystem=strict` / `ProtectHome`，
  仅 `/var/lib/labsentinel` 可写。
- 数据目录 `/var/lib/labsentinel/data`（`LS_DATA_DIR`），前端 `/opt/labsentinel/web`，
  模型 `/opt/labsentinel/models/`。
- 推理使用包内/系统 onnxruntime：把 `libonnxruntime.so` 放到可访问路径，用
  `ORT_DYLIB_PATH` 指定；ffmpeg 用 `LS_FFMPEG` 指定（`tools/ffmpeg/` 或系统包）。
- 日志：`journalctl -u labsentinel -f`。

## 三、验收清单

- [ ] `systemctl status labsentinel` active，`/api/health/ready` 返回 200
- [ ] `tools/e2e_smoke.py --base-url http://127.0.0.1:8000` 通过（文件源→事件；
      该脚本属训练/开发工具链，需 Python 环境）
- [ ] 4 路文件源 5fps 连续 30 分钟 RSS 稳定（`systemd-cgtop` / RSS 日志）
- [ ] 重启服务后数据/令牌保留（`/var/lib/labsentinel` 持久化生效）
