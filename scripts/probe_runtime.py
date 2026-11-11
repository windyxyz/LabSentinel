"""便携包/venv runtime 冒烟探针（2026-11-11 反误报治理）。

用法:
    python.exe probe_runtime.py [--app-dir <app/server 绝对路径>]

替代 build_portable.smoke()/deploy.py 体检里的 `python -c "import …"` 动态代码串——
卡巴斯基对「python.exe -c 动态代码串」启发式敏感（2026-11-09 install_portable.py
先例：内嵌 -c + urlopen + Popen 被判杀并删文件，重构为独立模块后稳定通过；
2026-11-11 staging 删除时间线与 build 冒烟的 -c 执行完全吻合）。改为文件执行后
不再命中该特征。本脚本不进包（构建机仓库文件，绝对路径调用）。

断言失败 → 非零退出 + stderr 说明；全部通过 → 逐行打印 OK 信息。
"""
from __future__ import annotations

import sys
from pathlib import Path


def main(argv: list[str]) -> int:
    # 2026-11-12：任何异常都落盘（stderr 在无控制台链下不可靠——此前启动失败
    # 只剩 returncode=1 黑箱）。正常路径不写文件。
    try:
        return _run(argv)
    except SystemExit:
        raise
    except BaseException:
        import os
        import traceback
        d = Path(os.environ.get("LS_DATA_DIR", ".")) / "logs"
        try:
            d.mkdir(parents=True, exist_ok=True)
            (d / "serve-boot-error.txt").write_text(traceback.format_exc(),
                                                    encoding="utf-8")
        except OSError:
            traceback.print_exc()
        return 1


def _run(argv: list[str]) -> int:
    app_dir: Path | None = None
    serve_port: int | None = None
    if "--app-dir" in argv:
        i = argv.index("--app-dir")
        app_dir = Path(argv[i + 1]).resolve()
        del argv[i:i + 2]
    if "--serve" in argv:
        # 便携包服务启动模式（2026-11-12 冒烟排障）：无控制台进程链下 stdout/stderr
        # 继承句柄不可靠，任何启动异常写 <LS_DATA_DIR>/logs/serve-boot-error.txt
        # 后以非零退出——launcher 能拿到确凿的失败原因而不是 returncode=1 黑箱。
        i = argv.index("--serve")
        serve_port = int(argv[i + 1])
        del argv[i:i + 2]
    if argv:
        raise SystemExit(f"[probe] 未知参数: {argv}")

    # 第一段：核心依赖导入（PATH 剥离环境下由调用方保证；覆盖 deploy.py 体检原
    # -c 串的依赖集：fastapi/uvicorn/cv2/numpy/httpx/supervision/pydantic_settings/
    # onnxruntime/grpc/cryptography）
    import fastapi  # noqa: F401
    import httpx  # noqa: F401
    import numpy  # noqa: F401
    import pydantic_settings  # noqa: F401
    import onnxruntime
    import supervision  # noqa: F401
    import cv2  # noqa: F401
    import uvicorn  # noqa: F401
    print("providers=", onnxruntime.get_available_providers())

    # 第二段：T-002 断言（paho/reportlab 可导入 + opencv 只有一份发行版，B-03 去重）
    # R2-001/N-01: grpc（sidecar）与 cryptography（密钥加密/更新验签）必须随锁进包
    import cryptography  # noqa: F401
    import grpc  # noqa: F401
    import paho.mqtt.client  # noqa: F401
    import reportlab  # noqa: F401
    import importlib.metadata as md
    dists = [x.metadata["Name"] for x in md.distributions()
             if x.metadata["Name"].lower().startswith("opencv")]
    assert len(dists) == 1, f"opencv 发行版不唯一: {dists}"
    print("paho/reportlab/grpc/cryptography OK, opencv dists=", dists)

    # 第三段：app.main 可导入（_pth 隔离模式不带 cwd；运行时由 uvicorn 自行插入
    # cwd，探针显式 insert 模拟）
    if app_dir is not None:
        sys.path.insert(0, str(app_dir))
        import app.main  # noqa: F401
        print("app.main import OK")

    if serve_port is not None:
        import os
        import traceback
        try:
            import uvicorn
            uvicorn.run("app.main:app",
                        host=os.environ.get("LS_HOST", "127.0.0.1"),
                        port=serve_port, log_level="info")
            return 0
        except BaseException:
            d = Path(os.environ.get("LS_DATA_DIR", ".")) / "logs"
            try:
                d.mkdir(parents=True, exist_ok=True)
                (d / "serve-boot-error.txt").write_text(traceback.format_exc(),
                                                        encoding="utf-8")
            except OSError:
                traceback.print_exc()
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
