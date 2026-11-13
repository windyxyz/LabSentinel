"""API 数据模型（与前端契约一致，见 web/）。"""
from datetime import datetime
from typing import Literal

from pydantic import BaseModel, Field, field_validator, model_validator


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

    @model_validator(mode="after")
    def _source_valid(self):
        # T-103（S-02/P0）：source 白名单校验——挡 exec:/ffmpeg:#raw/YAML 注入。
        # SourceError 是 ValueError，Pydantic 自动转 422；API 层统一转 400。
        from .sources import validate_source
        self.source = validate_source(self.type, self.source)
        return self


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


# ---- T-505/B-23：请求体模型（extra="forbid"——未知字段直接 422；
#      形状/长度/枚举约束由 Pydantic 自动 422，跨实体校验仍 400 由处理器负责） ----

class GroupBody(BaseModel):
    """任务111：分组名（空=未分组）。"""
    model_config = {"extra": "forbid"}
    group_name: str = Field("", max_length=32)


class BulkBody(BaseModel):
    """任务111：批量启停（去重校验留处理器 400；bool 冒充 int 显式拒绝——
    pydantic v2 默认会把 True 强转 1，与原处理器 isinstance(i, bool) 拒绝语义对齐）。"""
    model_config = {"extra": "forbid"}
    ids: list[int] = Field(min_length=1, max_length=100)
    action: Literal["start", "stop"]

    @field_validator("ids", mode="before")
    @classmethod
    def _no_bool(cls, v):
        if isinstance(v, list) and any(isinstance(x, bool) for x in v):
            raise ValueError("ids 必须是整数数组（不含 bool）")
        return v


class AnalyzerBody(BaseModel):
    """T-336：单路分析器实现切换/回退。"""
    model_config = {"extra": "forbid"}
    analyzer: Literal["python", "rust"]


class CopyConfigBody(BaseModel):
    """任务80：区域/规则批量复制（目标存在性/排除自身留处理器 400）。"""
    model_config = {"extra": "forbid"}
    target_ids: list[int] = Field(min_length=1)
    name_suffix: str = ""
    copy_zones: bool = True
    copy_rules: bool = True


class TemplateBody(BaseModel):
    """任务116：规则模板创建/更新（逐条规则校验留 _validate_template_rules 400）。"""
    model_config = {"extra": "forbid"}
    name: str = Field(min_length=1, max_length=64)
    rules: list[dict] = Field(min_length=1, max_length=50)


class TemplateApplyBody(BaseModel):
    """任务116：模板套用（zone 归属校验留处理器 400）。"""
    model_config = {"extra": "forbid"}
    camera_ids: list[int] = Field(min_length=1, max_length=100)
    zone_id: int | None = None


class CalendarBody(BaseModel):
    """任务112：假期/工作日例外（date 格式校验留处理器 400）。"""
    model_config = {"extra": "forbid"}
    date: str
    mode: Literal["holiday", "workday"]
    note: str = ""


class EventStatusBody(BaseModel):
    """任务12：单条处置（EVENT_STATUSES 之外由 Literal 直接 422）。"""
    model_config = {"extra": "forbid"}
    status: Literal["acked", "false_positive", "ignored"]
    handled_by: str = "admin"


class BulkStatusBody(BaseModel):
    """任务83：批量处置（单条失败容错逻辑留处理器）。"""
    model_config = {"extra": "forbid"}
    status: Literal["acked", "false_positive", "ignored"]
    ids: list[int] = Field(min_length=1, max_length=200)
    handled_by: str = "admin"

    @field_validator("ids", mode="before")
    @classmethod
    def _no_bool(cls, v):
        if isinstance(v, list) and any(isinstance(x, bool) for x in v):
            raise ValueError("ids 必须是整数数组（不含 bool）")
        return v


class ImportBody(BaseModel):
    """任务79：配置导入（format/version/规模校验留 _validate_config_import 400）。"""
    model_config = {"extra": "forbid"}
    mode: Literal["merge", "replace"]
    data: dict
    confirm_replace: bool = False


class SilenceCreate(BaseModel):
    """任务81：告警静默（scope 语义校验留处理器 400/404）。"""
    model_config = {"extra": "forbid"}
    scope: Literal["global", "camera", "level", "type"] = "global"
    target: str | int | None = None
    reason: str | None = None
    duration_s: int = Field(3600, ge=60, le=7 * 86400)


class TemplatePreviewBody(BaseModel):
    """任务85：告警模板预览样例事件。"""
    model_config = {"extra": "forbid"}
    ts: str | None = None
    camera_id: int = 1
    type: str = "enter"
    level: int = Field(2, ge=1, le=3)
    track_id: int = 1
    detail: str = "（示例事件详情）"


class ClaimPairingBody(BaseModel):
    """T-703：配对码换设备令牌（空 code → 设备侧 400 配对码无效）。"""
    model_config = {"extra": "forbid"}
    code: str
    device_name: str = Field("device", max_length=64)
    platform: str = Field("", max_length=32)


class DiscoveryProbeBody(BaseModel):
    """任务118：只读探测（凭证仅本次探测用）。"""
    model_config = {"extra": "forbid"}
    host: str = Field(min_length=1)
    port: int = Field(554, ge=1, le=65535)
    username: str | None = None
    password: str | None = None
