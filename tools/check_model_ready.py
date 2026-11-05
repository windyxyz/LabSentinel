"""模型就绪自检（任务32，docs/45-model-onboarding.md §3 上线门禁）。

只读检查：不启停服务、不做推理（onnxruntime 仅 CPU 建 session 读形状）、不改任何文件、
不打印 .env 敏感值（只判断非空/存在性）。

用法（m0-venv）:
    python tools/check_model_ready.py                       # 默认项目根 + v1 模型
    python tools/check_model_ready.py --model m0-spike/yolo11n_ppe.onnx --root .
    python tools/check_model_ready.py --help

输出 PASS/FAIL/WARN 表；退出码 0=可接入（PASS/WARN），1=存在 FAIL。
"""
import argparse
import csv
import glob
import sys
from datetime import datetime
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent


def parse_env(root: Path) -> dict:
    """极简 .env 解析（KEY=VALUE，忽略注释/空行）。只取本工具关心的键。"""
    env_path = root / "server" / ".env"
    out = {}
    if env_path.exists():
        for ln in env_path.read_text(encoding="utf-8", errors="replace").splitlines():
            ln = ln.strip()
            if ln and not ln.startswith("#") and "=" in ln:
                k, _, v = ln.partition("=")
                out[k.strip()] = v.strip().strip('"').strip("'")
    return out


def check_model_ready(root: Path, model: Path):
    rows = []  # (级别, 检查项, 明细, 修复建议)
    data_yaml = root / "datasets" / "merged" / "data.yaml"

    # 1. 文件存在性
    model_path = model if model.is_absolute() else root / model
    classes_path = model_path.with_suffix(".classes.txt")
    pt_path = model_path.with_suffix(".pt")
    rows.append(("PASS" if model_path.exists() else "FAIL", "ONNX 模型文件",
                 str(model_path), "先运行 tools/export_onnx.py（45 手册 §2）" if not model_path.exists() else ""))
    pt_exists = pt_path.exists()
    rows.append(("PASS" if pt_exists else "WARN", "训练权重 .pt（仅报告）",
                 f"{'存在: ' + str(pt_path) if pt_exists else '未找到（可选项）'}",
                 "训练产物未复制时运行 tools/train_ppe.py 会自动复制" if not pt_exists else ""))

    # 2. classes.txt 与 data.yaml
    yaml_names = []
    if data_yaml.exists():
        cfg = yaml.safe_load(data_yaml.read_text(encoding="utf-8"))
        yaml_names = cfg.get("names", [])
    n_cls_yaml = len(yaml_names)
    if classes_path.exists():
        vocab = [ln.strip() for ln in classes_path.read_text(encoding="utf-8").splitlines()
                 if ln.strip() and not ln.strip().startswith("#")]
        rows.append(("PASS", "classes.txt 读取", f"{len(vocab)} 类: {vocab}", ""))
        if yaml_names and vocab != yaml_names:
            diff = [f"{i}: txt={a!r} vs yaml={b!r}" for i, (a, b) in
                    enumerate(zip(vocab + [None] * n_cls_yaml, yaml_names + [None] * len(vocab)))
                    if a != b]
            rows.append(("WARN", "词表与 Resources/datasets/merged/data.yaml 不一致（顺序敏感）",
                         f"txt {len(vocab)} 类 vs yaml {n_cls_yaml} 类；差异: {diff}",
                         "v1(10类) 对合并数据集(11类) 属预期；v2 训后应一致（45 手册 §1 硬契约）"))
        else:
            rows.append(("PASS", "词表与 merged/data.yaml 一致", f"{n_cls_yaml} 类", ""))
    else:
        vocab = []
        rows.append(("FAIL", "classes.txt 缺失", str(classes_path),
                     "从 data.yaml names 生成同名文件（45 手册 §1；缺失时服务端回退 .env 词表）"))

    # 3. ONNX 结构（仅建 session 读形状，CPU，不推理）
    if model_path.exists():
        try:
            import onnxruntime as ort
            sess = ort.InferenceSession(str(model_path), providers=["CPUExecutionProvider"])
            inp = sess.get_inputs()[0]
            out = sess.get_outputs()[0]
            out_shape = out.shape[1] if len(out.shape) > 1 else None
            expect = 4 + len(vocab) if vocab else None
            shape_txt = f"in={tuple(inp.shape)} out={tuple(out.shape)}"
            if vocab and out_shape is not None:
                if out_shape == expect:
                    rows.append(("PASS", "ONNX 输出通道 = 4 + 类别数", f"{shape_txt}（{out_shape}={4}+{len(vocab)}）", ""))
                else:
                    rows.append(("FAIL", "ONNX 输出通道与词表不符",
                                 f"{shape_txt}；期望 {4 + len(vocab)}（4+{len(vocab)}）",
                                 "词表与模型不匹配：检查 classes.txt 是否对应该 ONNX（45 手册 §1）"))
            else:
                rows.append(("PASS", "ONNX 会话可创建", shape_txt, ""))
        except Exception as e:  # noqa: BLE001
            rows.append(("FAIL", "ONNX 加载失败", f"{type(e).__name__}: {e}",
                         "模型文件损坏或版本不兼容，重新导出（45 手册 §2）"))
        size_mb = model_path.stat().st_size / 1e6
        mtime = datetime.fromtimestamp(model_path.stat().st_mtime).strftime("%Y-%m-%d %H:%M")
        rows.append(("PASS", "模型元数据", f"{size_mb:.2f} MB, 修改于 {mtime}", ""))

    # 4. 配置校验（读 .env，不打印敏感值）
    env = parse_env(root)
    ppe_model_env = env.get("LS_PPE_MODEL")

    def _model_env_check():
        """返回 (级别, 明细, 修复建议)。"""
        if not ppe_model_env:
            return ("WARN", "（未设置，服务端用默认路径）", "v2 接入时按 45 手册 §3 设置")
        cand = Path(ppe_model_env)
        if not cand.is_absolute():
            cand = root / cand
        if cand.exists():
            return ("PASS", ppe_model_env, "")
        return ("WARN", f"{ppe_model_env}（路径不存在！服务端会静默回退到无 PPE 模式）",
                "修正 .env 的 LS_PPE_MODEL 路径")

    level, detail, fix = _model_env_check()
    rows.append((level, ".env 的 LS_PPE_MODEL", detail, fix))
    for key in ("LS_PPE_CLASS_NAMES", "LS_PPE_VIOLATION_CLASS"):
        val = env.get(key)
        if val:
            in_vocab = val.lower() in [v.lower() for v in vocab]
            rows.append(("PASS" if in_vocab else "FAIL", f".env 的 {key} 能在词表精确匹配（忽略大小写）",
                         val, "改成词表内的精确类名（PpeEngine 忽略大小写精确优先）" if not in_vocab else ""))
        else:
            rows.append(("WARN", f".env 未设置 {key}", "（服务端用默认值）", ""))
    rows.append(("PASS", ".env 敏感值未读取/未打印", "仅判断非空与路径存在性", ""))

    # 5. 训练结果元数据（只读）
    for csv_path in sorted(glob.glob(str(root / "Resources" / "runs" / "detect" / "*ppe_v2*" / "results.csv"))):
        try:
            with open(csv_path, encoding="utf-8") as f:
                r = list(csv.DictReader(f))
            if r:
                best = max(r, key=lambda x: float(x["metrics/mAP50(B)"]))
                rows.append(("PASS", f"训练记录 {Path(csv_path).parent.name}",
                             f"epoch {r[-1]['epoch']}/40，最佳 epoch{best['epoch']} "
                             f"mAP50={float(best['metrics/mAP50(B)']):.3f} "
                             f"mAP50-95={float(best['metrics/mAP50-95(B)']):.3f}", ""))
        except Exception as e:  # noqa: BLE001
            rows.append(("WARN", f"训练记录读取失败 {csv_path}", str(e), ""))

    return rows


def main():
    ap = argparse.ArgumentParser(description="模型就绪自检（只读；接入前一键核对，45 手册 §3 门禁）")
    ap.add_argument("--root", default=str(ROOT), help="项目根目录（默认脚本上级）")
    ap.add_argument("--model", default="m0-spike/yolo11n_ppe.onnx",
                    help="待检 ONNX（相对 --root 或绝对路径）")
    args = ap.parse_args()
    root = Path(args.root).resolve()

    rows = check_model_ready(root, Path(args.model))
    print("LabSentinel 模型就绪自检")
    print("=" * 72)
    for level, name, detail, fix in rows:
        mark = {"PASS": "PASS", "FAIL": "FAIL", "WARN": "WARN"}[level]
        print(f"[{mark}] {name}: {detail}" + (f"\n       ↳ 修复: {fix}" if fix and level == "FAIL" else ""))
    fails = [r for r in rows if r[0] == "FAIL"]
    warns = [r for r in rows if r[0] == "WARN"]
    print("=" * 72)
    print(f"结果：{'FAIL' if fails else 'PASS'}（FAIL {len(fails)} / WARN {len(warns)} / 总 {len(rows)} 项）"
          + ("" if not warns else "；WARN 不阻塞接入但建议处理"))
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
