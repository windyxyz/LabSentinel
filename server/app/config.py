"""LabSentinel 服务端配置。"""
from pathlib import Path

from pydantic_settings import BaseSettings

BASE_DIR = Path(__file__).resolve().parent.parent


class Settings(BaseSettings):
    # 模型：默认 FP32（DirectML/CPU 可直接吃）；NPU 后端自动切换 XINT8 版
    model_fp32: Path = BASE_DIR.parent / "m0-spike" / "yolo11n.onnx"
    model_int8: Path = BASE_DIR.parent / "m0-spike" / "yolo11n_xint8_hd.onnx"
    ppe_model: Path | None = None  # 实验服模型（Roboflow 数据集微调导出后配置）
    ppe_class_names: str = "lab coat"  # 逗号分隔，穿戴证据类别名
    ppe_violation_class: str = "no lab coat"  # 显式违规类别名（只有它才触发告警）
    ppe_conf: float = 0.30  # 违规判定阈值（高置信才告警）
    ppe_wearing_conf: float = 0.20  # 穿戴证据阈值（低置信即可证明合规）

    data_dir: Path = BASE_DIR / "data"
    db_path: Path = BASE_DIR / "data" / "labsentinel.db"
    snapshot_dir: Path = BASE_DIR / "data" / "snapshots"
    video_dir: Path = BASE_DIR / "data" / "videos"
    annotation_pool_dir: Path = BASE_DIR / "data" / "annotation_pool"  # 误报样本池（任务12）

    # 数据保留（任务14，合规：最小化存储）
    retention_events_days: int = 180  # 事件记录保留天数（含截图/视频文件）
    retention_pool_days: int = 90  # 误报标注池文件保留天数
    disk_min_free_gb: float = 5.0  # 磁盘低于此值触发紧急清理
    ready_min_free_gb: float = 1.0  # 就绪探针磁盘阈值（任务63，LS_READY_MIN_FREE_GB）
    sqlite_busy_timeout_ms: int = 5000  # 写锁等待窗口（任务64，LS_SQLITE_BUSY_TIMEOUT_MS；WAL 模式）
    gzip_max_bytes: int = 4 * 1024 * 1024  # gzip 大响应保护阈值（任务77，LS_GZIP_MAX_BYTES）
    escalate_after_min: int = 15  # 未处置升级提醒（任务82，LS_ESCALATE_AFTER_MIN；0=关闭）
    escalate_min_level: int = 3  # 只提醒 >= 此级别
    escalate_batch: int = 20  # 每轮最多处理条数（防启动洪峰）
    backup_interval_h: int = 24  # 自动备份间隔小时（任务92，0=关闭）
    backup_keep: int = 7  # 保留最近 N 份 auto 备份
    backup_include_media: bool = False  # true 时备份含媒体（大）
    security_headers: bool = True  # 安全响应头（任务89，LS_SECURITY_HEADERS）
    frame_options: str = "DENY"  # X-Frame-Options 值（DENY|SAMEORIGIN）
    csp: str = ""  # Content-Security-Policy（默认空=不发送）
    csp_report_only: bool = False  # true 时用 Report-Only 头（任务89）
    strict_config: bool = False  # 启动校验 FAIL 时拒绝启动（任务95，LS_STRICT_CONFIG）
    write_rate_limit: int = 120  # 每分钟写请求数上限（任务90，0=关闭）
    write_rate_window_s: int = 60  # 滑动窗口秒
    trust_proxy: bool = False  # 1 时信任 X-Forwarded-For 第一跳作为客户端 IP
    alert_template_title: str = "LabSentinel 级别{level}告警"  # 任务85：{level} 等占位符见 docs
    alert_template_text: str = ("{icon} L{level} {type_text} · {ts}\n"
                                "摄像头 #{camera_id}{track_part}\n{detail}")
    alert_filters: str = ""  # 任务86：JSON，按渠道过滤 {"wecom":{"min_level":2,"types":[..]}}
    pdf_font: str | None = None  # PDF 中文字体路径（默认 C:\Windows\Fonts\msyh.ttc）

    # 管理鉴权（任务16）：admin_token 为空 = 不启用（内网信任场景）；启用后写操作需 X-Admin-Token
    admin_token: str = ""
    auth_read_protected: bool = False  # true 时读操作也要求 token
    read_token: str = ""  # 只读令牌（任务88）：仅 admin_token 启用且读保护开启时可读；写永拒

    # go2rtc 流媒体网关（实时画面，见 app/go2rtc.py）
    go2rtc_api: str = "127.0.0.1:1984"  # go2rtc API 地址（本机）
    go2rtc_yaml: Path = BASE_DIR.parent / "tools" / "go2rtc.yaml"

    # 人脸比对（合规敏感，默认关闭；开启前必读 README 合规章节）
    face_enabled: bool = False
    compreface_url: str = "http://127.0.0.1:18000/api/v1"  # CompreFace api 服务地址
    compreface_api_key: str | None = None
    face_threshold: float = 0.75  # 1:N 识别相似度阈值
    face_cool_s: int = 30  # 同一 track 两次识别的最小间隔（秒）

    # 推理
    backend_order: str = "directml,vitisai,cpu"  # 依次尝试
    imgsz: int = 640
    conf_thres: float = 0.40
    iou_thres: float = 0.50
    analysis_fps: float = 5.0  # 每路分析帧率上限（检测不必逐帧）
    file_loop: bool = False  # 文件源循环播放（压测/演示用；默认关闭，正常播放完即止）
    min_box_area: int = 1200  # 过滤远处小框误检（像素面积，720p 下）

    # 图片文件夹接入（type=folder）：外部软件持续截图 → 自动识别关键图片
    folder_poll_s: float = 3.0  # 目录轮询间隔（秒）
    folder_stable_s: float = 1.5  # 文件修改后至少静置多久才读（防读到半张截图）
    folder_max_per_tick: int = 20  # 每轮最多处理张数（其余下一轮继续，防一次性打满）
    folder_recursive: bool = True  # 是否递归子目录
    folder_exts: str = ".jpg,.jpeg,.png,.bmp,.webp,.tif,.tiff"
    folder_max_file_mb: int = 32  # 超过此大小的图片跳过（防误指大文件目录）
    folder_full_rescan_s: int = 300  # 每 N 秒强制全量重扫（补抓"原地覆盖不改目录时间"的文件）
    folder_enter_events: bool = False  # true 时每张"有人进区"的图也按 enter 规则产生事件（默认只报显式违规）
    retention_images_days: int = 30  # 图片识别记录保留天数（原始截图永不删除）

    # 告警
    wecom_webhook: str | None = None
    dingtalk_webhook: str | None = None  # 钉钉机器人 webhook（仅 oapi.dingtalk.com）
    feishu_webhook: str | None = None  # 飞书群机器人 webhook（仅 open.feishu.cn）
    serverchan_sendkey: str | None = None  # Server酱 SendKey → 微信（sctapi.ftqq.com）
    pushplus_token: str | None = None  # PushPlus token → 微信（www.pushplus.plus）
    qmsg_key: str | None = None  # Qmsg酱 key → QQ（qmsg.zendee.cn）
    qmsg_qq: str | None = None  # Qmsg 接收消息的 QQ 号
    smtp_host: str | None = None
    smtp_port: int = 465
    smtp_user: str | None = None
    smtp_pass: str | None = None
    alert_email_to: str | None = None
    # 通用 Webhook（任务67）：推给自建服务；HMAC-SHA256 签名；默认仅 https
    webhook_url: str | None = None
    webhook_secret: str | None = None  # 配置后每个请求带 X-LS-Signature: sha256=<hex>
    webhook_allow_http: bool = False  # 局域网自建 http 服务显式放行
    webhook_timeout_s: float = 5.0

    # 单端口模式（任务43）：托管 web/dist，npm run build 后只开一个端口
    web_dist: Path = BASE_DIR.parent / "web" / "dist"

    class Config:
        env_prefix = "LS_"
        env_file = ".env"


settings = Settings()
settings.data_dir.mkdir(parents=True, exist_ok=True)
settings.snapshot_dir.mkdir(parents=True, exist_ok=True)
settings.video_dir.mkdir(parents=True, exist_ok=True)
settings.annotation_pool_dir.mkdir(parents=True, exist_ok=True)
