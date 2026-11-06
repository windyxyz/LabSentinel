"""T-505：路由拆分共享上下文（纯搬移期）。

api.py 在模块末尾把各 routers 子模块 include 进来；子模块只依赖本模块提供的
惰性代理（不在导入期读取 api 全局，避免 api ↔ routers 循环导入）。
"""
from __future__ import annotations

import sys
from typing import Any

from fastapi import Request


def _api() -> Any:
    """返回已加载的 app.api 模块（路由注册/请求时必然已加载）。"""
    mod = sys.modules.get("app.api")
    if mod is None:  # 极少见：包以别名导入
        import importlib
        mod = importlib.import_module("app.api")
    return mod


def store():
    """共享全局 Store（api.init 注入；测试夹具重建后即时可见）。"""
    return _api().store


def require_read(request):
    return _api().require_read(request)


def normalize_ts_range(start_ts, end_ts):
    return _api()._normalize_ts_range(start_ts, end_ts)


# 鉴权依赖（Depends 需要可调用对象本身，用同签名薄包装保持 OpenAPI 参数集合不变：
# request 必须带 fastapi.Request 注解，否则会被当成 query 参数暴露到契约里）
def read_guard(request: Request):
    return _api().require_read(request)


def admin_guard(request: Request):
    return _api().require_admin(request)


def triage_guard(request: Request):
    return _api().require_triage(request)


def face_client():
    return _api().face_client


def dispatcher():
    return _api().dispatcher


def render_alert(row):
    return _api().render_alert(row)


def alert_config():
    return _api().alert_config
