# LabSentinel Linux 部署指南（T-610）

> 两条路线：**Docker（推荐）** 与 **systemd 原生部署**。两者都只绑回环（T-101/Q-04）：
> 局域网访问须按 1.1.0 的 TLS + 设备配对（T-620）配置后再放开。

## 一、Docker（CPU / OpenVINO）

```bash
# 1) 准备模型（人体 yolo11n.onnx 必选；实验服模型按 Q-08 按需下载）
mkdir -p models && cp yolo11n.onnx models/

# 2) 首启令牌（Q-02）：写入环境变量文件
cat > .env <<'EOF'
LS_ADMIN_TOKEN=<用 python -c "import secrets;print(secrets.token_urlsafe(24))" 生成>
EOF

# 3) 起服务（cpu + go2rtc 两个容器，go2rtc 共享回环）
docker compose -f deploy/docker/compose.yaml --env-file .env up -d

# 4) 验证
curl -fsS http://127.0.0.1:8000/api/health/ready
curl -fsS -H "X-Admin-Token: $LS_ADMIN_TOKEN" http://127.0.0.1:8000/api/system
```

GPU 变体（NVIDIA，需 nvidia-container-toolkit）：

```bash
docker compose -f deploy/docker/compose.yaml \
  -f deploy/docker/compose.cuda.override.yaml up -d   # 使用 Dockerfile.cuda + TensorRT/CUDA 后端
```

要点：
- `/data` 卷持久化（DB/截图/视频/标注池）；模型目录只读挂载 `/models`。
- 容器内 `LS_HOST=127.0.0.1`；需要 LAN 时改为 `0.0.0.0` 并**同时**设置令牌与 TLS 终止代理。
- USB 摄像头透传：`--device /dev/video0`（容器内源填 `usb` + 设备号 0）。
- 镜像健康检查走 `/api/health/ready`（DB 完整性），30s 间隔。

## 二、systemd 原生部署

```bash
sudo useradd -r -s /usr/sbin/nologin labsentinel
sudo mkdir -p /opt/labsentinel /var/lib/labsentinel/data
sudo rsync -a server/ /opt/labsentinel/server/
python3.12 -m venv /opt/labsentinel/venv
/opt/labsentinel/venv/bin/pip install --require-hashes --no-deps \
  -r /opt/labsentinel/server/requirements-linux.lock
sudo mkdir -p /etc/labsentinel && sudo tee /etc/labsentinel/config.env >/dev/null <<'EOF'
LS_ADMIN_TOKEN=<令牌>
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

## 三、Rust 单二进制（T-333，可选）

```bash
python scripts/build_rust.py --zip        # 产物：Resources/dist/labsentinel-rust-*.zip
docker build -f deploy/docker/Dockerfile.rust -t labsentinel-server .
```

单二进制**不含 CPython 运行时**；模型目录仍走 `/models` 挂载。当前控制面已与 Python 契约对齐，
分析数据面（Rust sidecar）按 S7 迁移节奏逐步接管。

## 四、验收清单（§13.7）

- [ ] `docker compose up` 后 `/api/health/ready` 返回 200
- [ ] `tools/e2e_smoke.py --base-url http://127.0.0.1:8000` 通过（文件源→事件）
- [ ] 4 路文件源 5fps 连续 30 分钟 RSS 稳定（`docker stats`）
- [ ] 重启容器后数据/令牌保留（卷挂载生效）
- [ ] `docker scout cves` / `trivy image` 无 High 未处理项
