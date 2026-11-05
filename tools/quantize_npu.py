"""用 AMD Quark 把 YOLO ONNX 量化为 NPU（XDNA2/Ryzen AI）可用的 XINT8 模型。

!!! 必须用 npu-venv（Python 3.12）运行，m0-venv 里没有 amd-quark：
    D:\\LabSentinel\\npu-venv\\Scripts\\python.exe tools/quantize_npu.py
    D:\\LabSentinel\\npu-venv\\Scripts\\python.exe tools/quantize_npu.py --help

配方（docs/40-roadmap.md M0 实测，yolo11n NPU 40 FPS）:
  - Quark XINT8 + MinMax 校准，EnableNPUCnn/FoldRelu/AlignConcat 开
  - 必须排除检测头（/model.23 前缀子图，约 84 节点）——全图量化类别输出会归零
  - 校准集: Resources/datasets/lab-ppe/valid/images 抽 20 张
  - 本机无 Visual Studio MSVC -> 需要"注入绕道": 把 quark/onnx/operators/custom_ops/
    build_custom_ops.py 末尾的 _initialize_kernels() 调用剥掉后手工注入模块（仅内存中修改，
    不改 npu-venv 里的文件），再 import quark.onnx。正式环境建议装 VS Build Tools 走官方路径。
"""
import argparse
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_INPUT = ROOT / "m0-spike" / "yolo11n_ppe.onnx"
DEFAULT_OUTPUT = ROOT / "m0-spike" / "yolo11n_ppe_xint8.onnx"
DEFAULT_CALIB = ROOT / "Resources" / "datasets" / "lab-ppe" / "valid" / "images"
HEAD_PREFIX = "/model.23"  # YOLO 检测头子图前缀（Quark 导出的节点名以 /model.23/ 开头）


def parse_args():
    ap = argparse.ArgumentParser(
        description="AMD Quark XINT8 量化（NPU 用，检测头保留 FP32）。必须在 npu-venv（py3.12）下运行"
    )
    ap.add_argument("--input", default=str(DEFAULT_INPUT),
                    help=f"FP32 ONNX 输入（默认 {DEFAULT_INPUT}）")
    ap.add_argument("--output", default=str(DEFAULT_OUTPUT),
                    help=f"XINT8 输出路径（默认 {DEFAULT_OUTPUT}）")
    ap.add_argument("--calib-dir", default=str(DEFAULT_CALIB),
                    help=f"校准图片目录（默认 {DEFAULT_CALIB}）")
    ap.add_argument("--num-calib", type=int, default=20,
                    help="校准图片张数（默认 20，从目录列表中等距抽样）")
    ap.add_argument("--exclude-prefix", default=HEAD_PREFIX,
                    help=f"排除（保留 FP32）的节点名前缀（默认 {HEAD_PREFIX}，传 '' 可全图量化但不推荐）")
    ap.add_argument("--mode", default="XINT8", choices=["XINT8", "A8W8"],
                    help="量化模式：XINT8（NPU 专用配方，默认）或 A8W8（通用 INT8）")
    return ap.parse_args()


def patch_quark_import():
    """注入绕道：剥掉 build_custom_ops.py 末尾的 _initialize_kernels() 调用后手工加载模块。

    依据 docs/50-tasks/06-training-toolchain.md 附录（M0 实测有效）。只在内存中生效，
    不修改 npu-venv 里的任何文件。原因是该模块 import 时会 JIT 编译 C++ 自定义算子，
    本机无 Visual Studio MSVC 会直接失败。
    """
    import quark
    src_path = os.path.join(os.path.dirname(quark.__file__), "onnx", "operators",
                            "custom_ops", "build_custom_ops.py")
    with open(src_path, encoding="utf-8") as f:
        src = f.read()
    marker = "_initialize_kernels()"
    if marker in src:
        src = src[:src.rfind(marker)]  # 剥掉末尾调用（保留 def 定义）
        print(f"[info] 注入绕道: 已剥离 {os.path.basename(src_path)} 的 _initialize_kernels() 调用")
    else:
        print(f"[warn] {src_path} 中未找到 {marker}，可能 quark 版本已修复，直接注入原文件")
    pkg = "quark.onnx.operators.custom_ops.build_custom_ops"
    # 安全护栏：被加载的源文件必须是已安装 quark 包自身的文件（路径含 quark 包目录），
    # 防止该绕道被指向任意脚本。M0 实测方案（docs/40-roadmap.md），加载内容非用户输入。
    assert os.sep + "quark" + os.sep in src_path or "/quark/" in src_path,         f"注入源不在 quark 包内，拒绝加载: {src_path}"
    import importlib.util
    spec = importlib.util.spec_from_file_location(pkg, src_path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[pkg] = mod
    spec.loader.exec_module(mod)
    import quark.onnx  # noqa: F401  现在可用
    print("[info] quark.onnx 导入成功")


def collect_head_nodes(model_path: Path, prefix: str):
    """统计检测头节点（按 /model.23 前缀），用于日志与 exclude 名单。"""
    import onnx
    model = onnx.load(str(model_path))
    nodes = [n.name for n in model.graph.node if n.name.startswith(prefix)]
    total = len(model.graph.node)
    return model, total, nodes


def sample_images(calib_dir: Path, num: int):
    """从校准目录等距抽 num 张图（确定性，覆盖整个列表）。"""
    names = sorted(f for f in os.listdir(calib_dir)
                   if f.lower().endswith((".jpg", ".jpeg", ".png")))
    if not names:
        sys.exit(f"[error] 校准目录没有图片: {calib_dir}")
    if num >= len(names):
        picked = names
    else:
        step = len(names) / num
        picked = [names[min(int(i * step), len(names) - 1)] for i in range(num)]
    return [calib_dir / n for n in picked]


class ImageDataReader:
    """Quark 校准数据读取器约定: get_next() 返回 {input_name: ndarray} 或 None；支持 rewind/reset。"""

    def __init__(self, model_path, image_paths, imgsz=640):
        import cv2
        import numpy as np
        import onnxruntime as ort
        self._np, self._cv2 = np, cv2
        self.data_list = []
        for p in image_paths:
            img = self._imread(p)
            if img is None:
                print(f"[warn] 读取失败，跳过: {p}")
                continue
            img = cv2.resize(img, (imgsz, imgsz)).astype(np.float32)[:, :, ::-1]  # BGR->RGB
            img = img.transpose(2, 0, 1)[None] / 255.0                            # NCHW 0~1
            self.data_list.append(img)
        session = ort.InferenceSession(str(model_path), providers=["CPUExecutionProvider"])
        self.input_name = session.get_inputs()[0].name
        self.datasize = len(self.data_list)
        self.enum_data = None

    def _imread(self, path):
        """读图：项目路径含中文（监控识别），Windows 下 cv2.imread 对非 ASCII 路径会失败，
        回退到 np.fromfile + cv2.imdecode（与 ultralytics 的 imread 补丁同理）。"""
        cv2, np = self._cv2, self._np
        img = cv2.imread(str(path))
        if img is not None:
            return img
        buf = np.fromfile(str(path), dtype=np.uint8)
        if buf.size == 0:
            return None
        return cv2.imdecode(buf, cv2.IMREAD_COLOR)

    def get_next(self):
        if self.enum_data is None:
            self.enum_data = iter([{self.input_name: d} for d in self.data_list])
        return next(self.enum_data, None)

    def __len__(self):
        return self.datasize

    def rewind(self):
        self.enum_data = None

    def reset(self):
        self.enum_data = None


def check_weight_dtype(model_path, probes):
    """检查探针节点的卷积权重 dtype，验证检测头 FP32 / 主干 INT8。"""
    import onnx
    model = onnx.load(str(model_path))
    inits = {i.name: i for i in model.graph.initializer}
    for node_name in probes:
        node = next((n for n in model.graph.node if n.name == node_name), None)
        if node is None or len(node.input) < 2 or node.input[1] not in inits:
            print(f"[verify] {node_name}: 未找到权重初始化器，跳过 dtype 检查")
            continue
        dt = inits[node.input[1]].data_type
        dt_name = onnx.TensorProto.DataType.Name(dt)
        note = "FP32（未量化，符合预期）" if dt == onnx.TensorProto.FLOAT else "已量化"
        print(f"[verify] {node_name}: 权重 dtype={dt_name}（{note}）")


def main():
    args = parse_args()
    if sys.version_info[:2] != (3, 12):
        print(f"[warn] 当前 Python {sys.version_info[:2]}，本脚本按 npu-venv（py3.12 + amd-quark 0.12）验证")

    input_model = Path(args.input)
    if not input_model.exists():
        sys.exit(f"[error] 输入 ONNX 不存在: {input_model}（先运行 tools/export_onnx.py）")
    calib_dir = Path(args.calib_dir)
    if not calib_dir.is_dir():
        sys.exit(f"[error] 校准目录不存在: {calib_dir}（先运行 tools/download_datasets.py）")

    # 量化前统计（onnx 直接读图，报告用：FP32 总节点数 / 检测头节点数）
    _, fp32_nodes, head_nodes = collect_head_nodes(input_model, args.exclude_prefix)
    print(f"[info] FP32 模型总节点数: {fp32_nodes}；检测头（{args.exclude_prefix} 前缀）节点数: {len(head_nodes)}")

    patch_quark_import()
    from quark.onnx import CalibMethod, ModelQuantizer, QConfig, QLayerConfig

    if args.mode == "XINT8":
        from quark.onnx import XInt8Spec
        act = wgt = XInt8Spec()
        extra = {"EnableNPUCnn": True, "AlignSlice": False, "FoldRelu": True, "AlignConcat": True}
    else:
        from quark.onnx import Int8Spec
        act = wgt = Int8Spec()
        extra = {"EnableNPUCnn": False, "AlignSlice": False, "FoldRelu": True, "AlignConcat": True}
    act.set_calibration_method(CalibMethod.MinMax)
    wgt.set_calibration_method(CalibMethod.MinMax)

    # exclude：精确节点名 + 正则双保险（quark 支持 ^ 开头含 .* 的正则；预处理若改名仍能命中）
    exclude = ["^" + re.escape(args.exclude_prefix) + "/.*"] if args.exclude_prefix else []
    exclude += head_nodes
    if exclude:
        print(f"[info] 排除检测头: {len(head_nodes)} 个节点（前缀 {args.exclude_prefix}），保留 FP32")

    images = sample_images(calib_dir, args.num_calib)
    print(f"[info] 校准图: {len(images)}/{len(list(calib_dir.iterdir()))} 张（等距抽样 from {calib_dir.name}）")
    reader = ImageDataReader(input_model, images)
    print(f"[info] 校准样本数: {len(reader)}，模型输入名: {reader.input_name}")

    qconfig = QConfig(
        global_config=QLayerConfig(activation=act, weight=wgt),
        algo_config=[],
        use_external_data_format=False,
        exclude=exclude,
        **extra,
    )
    quantizer = ModelQuantizer(qconfig)
    print(f"[info] 开始量化 {args.mode}: {input_model.name} -> {Path(args.output).name}")
    quantizer.quantize_model(str(input_model), str(args.output), reader)

    # 量化后验证与节点数对比
    out = Path(args.output)
    _, xint8_nodes, _ = collect_head_nodes(out, args.exclude_prefix)
    print(f"[done] XINT8 模型: {out}（{out.stat().st_size / 1e6:.2f} MB）")
    print(f"[done] 节点数对比: FP32 {fp32_nodes} -> XINT8 {xint8_nodes}（量化会插入量化/反量化节点）")
    if args.exclude_prefix:
        # 探针: 检测头第一个分支卷积（应保持 FP32）+ 主干第一个卷积（应已量化 INT8）
        head_probe = next((n for n in head_nodes
                           if n.startswith(args.exclude_prefix + "/cv2.0") and n.endswith("conv/Conv")),
                          head_nodes[0] if head_nodes else None)
        probes = [p for p in (head_probe, "/model.0/conv/Conv") if p]
        if probes:
            check_weight_dtype(out, probes)
    print("[next] NPU 推理验证参考 m0-spike/s2c_fps_npu.py（onnxruntime-vitisai, VitisAIExecutionProvider）")


if __name__ == "__main__":
    main()
