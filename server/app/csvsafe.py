"""T-502（S-15）：CSV 公式注入防护——Excel 公式注入统一前缀（api._csv_cell 与
export 事件 CSV 共用，防两套规则漂移；文件夹 CSV 早已处理，此处补齐事件/统计导出）。"""

_DANGEROUS = ("=", "+", "-", "@", "\t")


def safe_cell(v) -> str:
    """公式注入防护：以 =+-@tab 开头的单元格前置单引号（Excel 按 文本 处理）。"""
    s = "" if v is None else str(v)
    if s.startswith(_DANGEROUS):
        return "'" + s
    return s
