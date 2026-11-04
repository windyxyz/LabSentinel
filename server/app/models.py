"""API 数据模型（与前端契约一致，见 web/）。"""
from datetime import datetime

from pydantic import BaseModel, Field


class Camera(BaseModel):
    id: int
    name: str
    source: str
    type: str  # rtsp | file | usb | folder（图片文件夹自动识别）
    status: str = "stopped"  # running | stopped | error


class CameraCreate(BaseModel):
    name: str
    source: str
    type: str = "file"
    autostart: bool = True  # 置 false 可先配安全区/规则再手动启动
    group_name: str = Field("", max_length=32)  # 任务111：分组（空=未分组）


class Zone(BaseModel):
    id: int
    camera_id: int
    name: str
    polygon: list[list[int]]  # [[x,y], ...] 1280x720 像素坐标


class ZoneCreate(BaseModel):
    camera_id: int
    name: str
    polygon: list[list[int]]


class Rule(BaseModel):
    id: int
    camera_id: int
    zone_id: int | None = None
    behavior: str  # enter_no_ppe | enter | exit | occupancy_over
    level: int = 2  # 1 提示 2 警告 3 严重
    cooldown_s: int = 60
    time_start: str = "00:00"
    time_end: str = "23:59"
    threshold: int = 0  # occupancy_over 用：人数上限
    enabled: bool = True


class RuleCreate(BaseModel):
    camera_id: int
    zone_id: int | None = None
    behavior: str
    level: int = Field(2, ge=1, le=3)  # 2026-10-30 审阅：入库即校验（原 level=99 也能入库）
    cooldown_s: int = Field(60, ge=0)
    time_start: str = "00:00"
    time_end: str = "23:59"
    threshold: int = Field(0, ge=0)  # occupancy_over 用：人数上限
    enabled: bool = True
    days_of_week: str = "1,2,3,4,5,6,7"  # 任务112：ISO 星期集合（1=周一…7=周日），空=不生效
    holiday_only: bool = False  # 任务112：true=只在假期日期生效（见 /api/calendar 例外表）


class Event(BaseModel):
    id: int
    ts: datetime
    camera_id: int
    type: str
    level: int
    track_id: int
    detail: str
    has_snapshot: bool
