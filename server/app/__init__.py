"""LabSentinel 服务端包。

注意：本文件在所有子模块之前执行。OpenH264（Cisco，浏览器可播的 H.264 编码）
由 cv2 在创建 VideoWriter 时按"导入时快照的搜索路径"动态加载，因此必须在
import cv2 之前把 cv2 包目录放进 PATH。获取方式见 tools/README.md。
"""
import os
import sysconfig
from pathlib import Path

_cv2_dir = Path(sysconfig.get_paths()["purelib"]) / "cv2"
if _cv2_dir.is_dir() and str(_cv2_dir) not in os.environ.get("PATH", ""):
    os.environ["PATH"] = str(_cv2_dir) + os.pathsep + os.environ.get("PATH", "")
