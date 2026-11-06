# T-610（Q-10）：LabSentinel 服务端 Docker 镜像——CPU 变体（含 OpenVINO EP 可选）。
# 多阶段：web 构建 → Python 运行时（锁文件安装）→ 非 root + /data 卷 + 健康探针。
FROM node:20-bookworm-slim AS web
WORKDIR /w
COPY web/package.json web/package-lock.json ./
RUN npm ci --registry=https://registry.npmmirror.com
COPY web/ ./
RUN npm run build

FROM python:3.12-slim-bookworm AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
        ffmpeg libgl1 libglib2.0-0 curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 10001 -d /data labsentinel
WORKDIR /app/server
COPY server/requirements-linux.lock ./
RUN pip install --no-cache-dir --require-hashes --no-deps -r requirements-linux.lock
COPY server/app ./app
COPY --from=web /w/dist /app/web
COPY VERSION /app/VERSION
COPY NOTICE /app/NOTICE
ENV LS_DATA_DIR=/data \
    LS_DB_PATH=/data/labsentinel.db \
    LS_SNAPSHOT_DIR=/data/snapshots \
    LS_VIDEO_DIR=/data/videos \
    LS_ANNOTATION_POOL_DIR=/data/annotation_pool \
    LS_WEB_DIST=/app/web \
    LS_BACKEND_ORDER=openvino,cpu \
    LS_HOST=127.0.0.1 \
    PYTHONUTF8=1
RUN mkdir -p /data/snapshots /data/videos /data/annotation_pool \
    && chown -R labsentinel:labsentinel /data /app
USER labsentinel
VOLUME ["/data"]
EXPOSE 8000
HEALTHCHECK --interval=30s --timeout=5s \
  CMD curl -fsS http://127.0.0.1:8000/api/health/ready || exit 1
CMD ["python", "-m", "uvicorn", "app.main:app", "--host", "127.0.0.1", "--port", "8000"]
