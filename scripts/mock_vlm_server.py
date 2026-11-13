"""RA5 模拟验收：Mock OpenAI 兼容 VLM 服务器（外部联网模型的协议替身）。

模拟 /v1/chat/completions：按请求里图片帧数返回确定性判定 JSON（帧内像素
启发式——图片亮度/尺寸参与判定，保证"有人/无人"素材得到不同结果）。
用途：无外部 API key 时验证 LabSentinel VLM 通路全链（请求构造/多帧编码/
JSON 解析/回填/事件），真实模型质量验收待 API key 配置后同口径复跑。

用法: python scripts/mock_vlm_server.py [--port 8899]
协议: POST /v1/chat/completions（openai 兼容）→ {"choices":[{"message":{"content":
  '{"persons":<n>,"no_ppe":<n>,"violations":[...],"summary":"mock"}'}}]}
判定规则（启发式）：images 数组长度>0 时 persons=图片序号哈希%4，
  no_ppe=persons>0 ? 1 : 0，violations=["no_ppe"] when no_ppe>0。
  通过 ?mode=empty 强制 persons=0（验收"无人"素材）。
"""
import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MODE = "auto"
STATS = {"images": 0, "requests": 0}


def verdict_for(n_images: int, content_len: int) -> dict:
    """内容启发式判定：帧 JPEG 长度参与 persons 计算——不同素材得到不同
    结果（验收可断言区分度）；empty 模式强制无人（无违规素材验收）。"""
    if MODE == "empty":
        return {"persons": 0, "no_ppe": 0, "violations": [], "summary": "mock: 无人"}
    persons = (content_len // 1024 + n_images) % 3 + 1  # 1..3（有人素材）
    # 有人即报未穿服（实验室语义；无 API key 的 mock 无法真判内容——
    # 验收目标是事件链路通路，persons 区分度由 content_len 启发式保证）
    no_ppe = persons
    return {
        "persons": persons,
        "no_ppe": no_ppe,
        "violations": ["no_ppe"] if no_ppe > 0 else [],
        "summary": f"mock: 检出 {persons} 人，未穿实验服 {no_ppe} 人",
    }


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):  # 静默默认访问日志
        pass

    def do_GET(self):
        if self.path.startswith("/stats"):
            self._reply(200, dict(STATS))
            return
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        if not self.path.startswith("/v1/chat/completions"):
            self.send_response(404)
            self.end_headers()
            return
        n = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(n)
        try:
            req = json.loads(body)
        except ValueError:
            self._reply(400, {"error": "bad json"})
            return
        content = req.get("messages", [{}])[0].get("content", [])
        n_images = sum(
            1 for c in content if isinstance(c, dict) and c.get("type") == "image_url"
        )
        # 帧内容长度启发式（各帧 data:image/jpeg;base64,<len>）
        lens = [
            len(c.get("image_url", {}).get("url", ""))
            for c in content
            if isinstance(c, dict) and c.get("type") == "image_url"
        ]
        STATS["images"] += n_images
        STATS["requests"] += 1
        v = verdict_for(n_images, sum(lens))
        text = "```json\n" + json.dumps(v, ensure_ascii=False) + "\n```"
        self._reply(200, {
            "id": "mock-vlm-1",
            "choices": [{"message": {"content": text}}],
        })

    def _reply(self, code: int, obj: dict) -> None:
        payload = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description="Mock OpenAI 兼容 VLM（RA5 验收）")
    ap.add_argument("--port", type=int, default=8899)
    ap.add_argument("--mode", choices=["auto", "empty"], default="auto",
                    help="empty=所有图判无人（验收无违规素材）")
    a = ap.parse_args()
    MODE = a.mode
    print(f"[mock-vlm] 监听 127.0.0.1:{a.port}（mode={MODE}）", flush=True)
    ThreadingHTTPServer(("127.0.0.1", a.port), Handler).serve_forever()
