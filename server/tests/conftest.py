# 测试会话全局：关闭文件日志（避免 pytest 污染真实 server/data/logs）。
# 单测文件日志的用例自行 monkeypatch LS_LOG_FILE 与 settings.data_dir（见 test_logging_diag.py）。
import os

os.environ.setdefault("LS_LOG_FILE", "0")
