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
    pdf_font: str | None = None  # PDF 中文字体路径（默认 C:\Windows\Fonts\msyh.ttc）

    # 管理鉴权（任务16）：admin_token 为空 = 不启用（内网信任场景）；启用后写操作需 X-Admin-Token
    admin_token: str = ""
    auth_read_protected: bool = False  # true 时读操作也要求 token

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

    class Config:
        env_prefix = "LS_"
        env_file = ".env"


settings = Settings()
settings.data_dir.mkdir(parents=True, exist_ok=True)
settings.snapshot_dir.mkdir(parents=True, exist_ok=True)
settings.video_dir.mkdir(parents=True, exist_ok=True)
settings.annotation_pool_dir.mkdir(parents=True, exist_ok=True)
