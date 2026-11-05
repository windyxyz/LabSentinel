"""T-102（S-01/F-01）：WebRTC 信令的 FastAPI 鉴权代理。

浏览器不再直连 go2rtc :1984（控制面带 RTSP 凭证、暴露全部画面）；
改为连接 /ws/webrtc/{cid}（同源，走 FastAPI 鉴权），本模块把 SDP/ICE 文本帧
双向转发到 127.0.0.1:1984 的 go2rtc（Basic 鉴权）。媒体面仍在 :8555。
"""
import asyncio

import websockets
from fastapi import WebSocket, WebSocketDisconnect

from . import go2rtc

_UPSTREAM = "ws://127.0.0.1:1984/api/ws"


async def proxy(ws: WebSocket, cid: int) -> None:
    """双向透传文本帧（SDP/ICE 都是 JSON 文本）；任一侧断开即清理另一侧。"""
    user, pwd = go2rtc._secret()
    import base64
    auth = "Basic " + base64.b64encode(f"{user}:{pwd}".encode()).decode()
    url = f"{_UPSTREAM}?src={cid}"
    async with websockets.connect(url, additional_headers={"Authorization": auth},
                                  open_timeout=5, max_size=1 << 20) as up:
        async def c2u():
            while True:
                await up.send(await ws.receive_text())

        async def u2c():
            async for msg in up:
                await ws.send_text(msg if isinstance(msg, str) else msg.decode("utf-8", "replace"))

        tasks = [asyncio.create_task(c2u()), asyncio.create_task(u2c())]
        try:
            done, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            for t in done:
                t.result()  # 暴露异常（上游断流等）
        finally:
            for t in tasks:
                t.cancel()
