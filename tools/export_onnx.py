"""把训练好的 best.pt 导出为 ONNX（opset 13，imgsz 640），供 LabSentinel 推理。

默认: models/yolo11n_ppe.pt -> models/yolo11n_ppe.onnx（server/.env 的 LS_PPE_MODEL 指向它）。
导出后打印 10 类别顺序——PPE 引擎（PpeEngine）按类别名匹配，依赖 data.yaml 的 names 顺序。

用法（m0-venv）:
    D:\\LabSentinel\\m0-venv\\Scripts\\python.exe tools/export_onnx.py
    D:\\LabSentinel\\m0-venv\\Scripts\\python.exe tools/export_onnx.py --weights Resources/runs/detect/runs/ppe_v1/weights/best.pt
    D:\\LabSentinel\\m0-venv\\Scripts\\python.exe tools/export_onnx.py --help

导出成功后可用 tools/quantize_npu.py（npu-venv）进一步做 NPU XINT8 量化。
"""
import argparse
import json
import os
import shutil
import sys
from pathlib import Path

# 必须在 import ultralytics 之前设置：ultralytics 导出时会 AutoUpdate 缺失的
# onnxruntime（CPU 版）——它与本机 m0-venv 的 onnxruntime-directml 互斥（铁律），
# 误装会损坏推理环境。关闭 AutoUpdate，缺包时给出手动安装指引。
os.environ.setdefault("YOLO_AUTOINSTALL", "false")

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_WEIGHTS = ROOT / "models" / "yolo11n_ppe.pt"
DEFAULT_OUT = ROOT / "models" / "yolo11n_ppe.onnx"
DEFAULT_DATA = ROOT / "Resources" / "datasets" / "lab-ppe" / "data.yaml"


def parse_args():
    ap = argparse.ArgumentParser(
        description="best.pt -> FP32 ONNX（opset 13，imgsz 640），打印类别顺序并用 onnxruntime 验证输入形状"
    )
    ap.add_argument("--weights", default=str(DEFAULT_WEIGHTS),
                    help=f"输入 .pt 权重（默认 {DEFAULT_WEIGHTS}）")
    ap.add_argument("--out", default=str(DEFAULT_OUT),
                    help=f"输出 ONNX 路径（默认 {DEFAULT_OUT}）")
    ap.add_argument("--imgsz", type=int, default=640, help="输入尺寸（默认 640）")
    ap.add_argument("--opset", type=int, default=13, help="ONNX opset（默认 13，NPU 量化要求）")
    ap.add_argument("--data", default=str(DEFAULT_DATA),
                    help=f"data.yaml 路径，用于交叉核对类别顺序（默认 {DEFAULT_DATA}，--data '' 跳过核对）")
    ap.add_argument("--skip-verify", action="store_true",
                    help="跳过导出后的 onnxruntime 输入形状验证")
    ap.add_argument("--half", action="store_true",
                    help="LS-056：导出 FP16 ONNX（DirectML FP16 提速；CPU 推理需 EP 支持）")
    ap.add_argument("--arch", choices=["yolo", "rtdetr"], default="yolo",
                    help="输入权重架构（默认 yolo）")
    return ap.parse_args()


def check_names(names: dict, data_yaml: str):
    """与 data.yaml 的 names 列表核对类别顺序（PPE 引擎按名称匹配依赖此顺序）。"""
    if not data_yaml or not Path(data_yaml).exists():
        print("[warn] 未找到 data.yaml，跳过类别顺序核对")
        return
    import yaml
    with open(data_yaml, encoding="utf-8") as f:
        cfg = yaml.safe_load(f)
    yaml_names = cfg.get("names")
    if isinstance(yaml_names, dict):
        yaml_names = [yaml_names[i] for i in sorted(yaml_names)]
    model_names = [names.get(i) for i in range(len(names))]
    if model_names == yaml_names:
        print(f"[verify] 类别顺序与 {data_yaml} 的 names 完全一致（{len(model_names)} 类）")
    else:
        print(f"[warn] 类别顺序与 {data_yaml} 不一致！模型: {model_names}  yaml: {yaml_names}")


def mark_rtdetr_export(out: Path, nc: int):
    import onnx

    graph = onnx.load(out)
    metadata = {entry.key: entry.value for entry in graph.metadata_props}
    dims = graph.graph.output[0].type.tensor_type.shape.dim
    if len(dims) != 3:
        sys.exit(f"[error] RT-DETR 输出必须是 [batch, queries, channels]，实得 {len(dims)} 维")
    columns = dims[-1].dim_value
    if columns == 6 and (metadata.get("head") == "RTDETRDecoder" or nc != 2):
        output_format = "rtdetr_topk"
    elif columns == 4 + nc:
        output_format = "rtdetr_scores"
    else:
        sys.exit(f"[error] 不支持的 RT-DETR 输出列数 {columns}")
    metadata["labsentinel_output_format"] = output_format
    onnx.helper.set_model_props(graph, metadata)
    onnx.save(graph, out)
    sidecar = out.with_suffix(".model.json")
    settings = json.loads(sidecar.read_text(encoding="utf-8")) if sidecar.exists() else {}
    sidecar.write_text(json.dumps({**settings, "output_format": output_format}, indent=2) + "\n", encoding="utf-8")
    print(f"[verify] RT-DETR 输出格式: {output_format}")


def main():
    args = parse_args()
    weights = Path(args.weights)
    if not weights.exists():
        sys.exit(f"[error] 权重不存在: {weights}（先运行 tools/train_ppe.py 或用 --weights 指定已有权重）")

    # 依赖预检（AutoUpdate 已关闭，缺包时手动安装，勿污染 venv）
    import importlib.util
    required = ["onnx", "onnxslim"]
    if args.half:
        required.append("onnxconverter_common")
    missing = [m for m in required if importlib.util.find_spec(m) is None]
    if missing:
        sys.exit(f"[error] 缺少依赖 {missing}，请手动安装（勿让 ultralytics 自动装 onnxruntime，与 directml 互斥）:\n"
                 f"        {sys.executable} -m pip install {' '.join(missing)} -i https://pypi.tuna.tsinghua.edu.cn/simple")

    from ultralytics import RTDETR, YOLO

    model = RTDETR(str(weights)) if args.arch == "rtdetr" else YOLO(str(weights))
    names = model.names
    print(f"[info] 模型类别（nc={len(names)}，顺序即输出通道顺序，务必与 data.yaml names 一致）:")
    for i in range(len(names)):
        print(f"  {i}: {names[i]}")
    check_names(names, args.data)

    print(f"[info] 导出 FP32 ONNX: {weights.name} -> opset={args.opset} imgsz={args.imgsz}")
    exported = Path(model.export(format="onnx", imgsz=args.imgsz, opset=args.opset, half=False))
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    if args.half:
        import onnx
        from onnxconverter_common import float16
        converted = float16.convert_float_to_float16(onnx.load(exported), keep_io_types=False)
        onnx.checker.check_model(converted)
        onnx.save(converted, out)
    elif exported.resolve() != out.resolve():
        shutil.copy2(exported, out)
        print(f"[info] 导出产物 {exported} 已复制到 {out}")
    if args.arch == "rtdetr":
        mark_rtdetr_export(out, len(names))
    # 导出后同步生成 classes.txt 与 SHA256
    classes_file = out.with_name(out.stem + ".classes.txt")
    try:
        import hashlib
        names = model.names if isinstance(model.names, dict) else {}
        if names:
            text = "\n".join(str(names[i]) for i in sorted(names)) + "\n"
            classes_file.write_text(text, encoding="utf-8")
        digest = hashlib.sha256(out.read_bytes()).hexdigest()
        (out.with_name(out.stem + ".sha256.txt")).write_text(
            digest + "  " + out.name + "\n", encoding="utf-8"
        )
        print(f"[info] 类别表: {classes_file}；SHA256: {digest}")
    except Exception as e:  # 类别/哈希生成失败不阻断导出
        print(f"[warn] classes/sha256 生成失败: {e}")
    print(f"[done] {'FP16' if args.half else 'FP32'} ONNX: {out}（{out.stat().st_size / 1e6:.2f} MB）")

    if not args.skip_verify:
        import onnxruntime as ort
        sess = ort.InferenceSession(str(out), providers=["CPUExecutionProvider"])
        inp = sess.get_inputs()[0]
        shape = tuple(inp.shape)
        print(f"[verify] onnxruntime 加载成功（providers={sess.get_providers()}）")
        print(f"[verify] 输入: {inp.name} shape={shape} dtype={inp.type}")
        expect = (1, 3, args.imgsz, args.imgsz)
        if shape != expect:
            sys.exit(f"[error] 输入形状 {shape} 与预期 {expect} 不符！")
        expected_type = "tensor(float16)" if args.half else "tensor(float)"
        if inp.type != expected_type:
            sys.exit(f"[error] 输入类型 {inp.type} 与预期 {expected_type} 不符！")
        print(f"[verify] 输入形状 {shape} == {expect}，OK")
        outs = sess.get_outputs()[0]
        print(f"[verify] 输出: {outs.name} shape={tuple(outs.shape)}")


if __name__ == "__main__":
    main()
