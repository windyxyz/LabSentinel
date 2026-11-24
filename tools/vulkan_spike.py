"""LS-086 forward-pass comparison. Runs in an isolated development environment."""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
from threading import Barrier
from time import perf_counter

import numpy as np


def input_tensor(path):
    frame = np.fromfile(path, dtype=np.uint8).reshape(720, 1280, 3)
    # Fixed 2:1 resize and identical 114 padding for all candidates.
    reduced = frame.astype(np.uint16).reshape(360, 2, 640, 2, 3).sum(axis=(1, 3))
    canvas = np.full((640, 640, 3), 114, dtype=np.uint8)
    canvas[140:500] = (reduced + 2) // 4
    return np.ascontiguousarray(canvas[:, :, ::-1].transpose(2, 0, 1)[None], dtype=np.float32) / 255


def ort_factory(model, runtime, adapter, allow_cpu_ops=False):
    import onnxruntime as ort

    gpu = runtime == "dawn"
    if gpu:
        import onnxruntime_ep_webgpu as plugin

        if os.name == "nt":
            import ctypes

            ctypes.WinDLL(str(Path(os.environ["WINDIR"]) / "System32" / "vulkan-1.dll"))
        ort.register_execution_provider_library(plugin.get_ep_name(), plugin.get_library_path())
        device = next(d for d in ort.get_ep_devices() if d.ep_name == plugin.get_ep_name()
                      and d.device.type == ort.OrtHardwareDeviceType.GPU)

    def load(profile=None):
        opts = ort.SessionOptions()
        opts.intra_op_num_threads = 2
        if profile is not None:
            opts.enable_profiling = True
            opts.profile_file_prefix = str(profile)
        if gpu:
            opts.add_provider_for_devices(
                [device], {"dawnBackendType": "Vulkan", "adapterIndex": str(adapter)}
            )
            if not allow_cpu_ops:
                opts.add_session_config_entry("session.disable_cpu_ep_fallback", "1")
            session = ort.InferenceSession(str(model), sess_options=opts)
            if session.get_providers()[0] != plugin.get_ep_name():
                raise RuntimeError(f"WebGPU did not load: {session.get_providers()}")
        else:
            session = ort.InferenceSession(str(model), sess_options=opts, providers=["CPUExecutionProvider"])
        name = session.get_inputs()[0].name

        def infer(frame):
            return session.run(None, {name: frame})[0]

        if profile is not None:
            return infer, session.end_profiling
        return infer

    return load, (f"Dawn/Vulkan requested (adapter {adapter})" if gpu else f"ORT {ort.__version__} CPU")


def ncnn_factory(model, adapter, fp16):
    import ncnn

    if not 0 <= adapter < ncnn.get_gpu_count():
        raise RuntimeError(f"Vulkan adapter {adapter} is unavailable")
    device = ncnn.get_gpu_info(adapter).device_name()

    def load():
        net = ncnn.Net()
        net.opt.use_vulkan_compute = True
        net.opt.use_fp16_packed = fp16
        net.opt.use_fp16_storage = fp16
        net.opt.use_fp16_arithmetic = fp16
        net.opt.num_threads = 2
        net.set_vulkan_device(adapter)
        if net.load_param(f"{model}.param") or net.load_model(f"{model}.bin"):
            raise RuntimeError(f"cannot load {model}.param and .bin")
        input_name, = net.input_names()
        output_name, = net.output_names()

        def infer(frame):
            with net.create_extractor() as extractor:
                if extractor.input(input_name, ncnn.Mat(frame, batch_index=0).clone()):
                    raise RuntimeError("ncnn rejected input")
                status, result = extractor.extract(output_name)
                if status:
                    raise RuntimeError(f"ncnn extract failed with status {status}")
                return result.numpy(batch_index=0).copy()

        return infer

    return load, f"ncnn/Vulkan ({device}, {'FP16 enabled' if fp16 else 'FP32'})"


def overlap(box, other):
    extent = np.maximum(0, np.minimum(box[2:], other[2:]) - np.maximum(box[:2], other[:2]))
    intersection = np.prod(extent)
    return intersection / (np.prod(box[2:] - box[:2]) + np.prod(other[2:] - other[:2]) - intersection + 1e-7)


def boxes(output):
    rows = output[0].T
    winners = []
    for class_id in range(rows.shape[1] - 4):
        scores = rows[:, class_id + 4]
        indices = np.flatnonzero(scores >= 0.25)
        kept = []
        for index in indices[np.argsort(-scores[indices], kind="stable")]:
            center, half = rows[index, :2], rows[index, 2:4] / 2
            box = np.concatenate((center - half, center + half))
            if all(overlap(box, previous) <= 0.45 for previous in kept):
                kept.append(box)
                winners.append((class_id, float(scores[index]), box))
    return winners


def agreement(reference, candidate):
    if reference.shape != candidate.shape or not np.isfinite(candidate).all():
        raise RuntimeError(f"invalid output {candidate.shape}, expected {reference.shape}")
    expected, actual = boxes(reference), boxes(candidate)
    matches = 0
    for label, score, box in expected:
        overlaps = [overlap(box, bbox) if cid == label and abs(conf - score) <= 0.02 else 0
                    for cid, conf, bbox in actual]
        best = int(np.argmax(overlaps)) if overlaps else 0
        if overlaps and overlaps[best] >= 0.95:
            matches += 1
            actual.pop(best)
    return {
        "expected_boxes": len(expected), "matched_boxes": matches, "extra_boxes": len(actual),
        "max_score_delta": float(np.max(np.abs(reference[:, 4:] - candidate[:, 4:]))),
    }


def measure(load, frames, streams, iterations):
    runners = [load() for _ in range(streams)]
    for infer in runners:
        for frame in frames:
            infer(frame)
    barrier = Barrier(streams + 1)

    def run(infer):
        barrier.wait()
        samples = []
        for i in range(iterations):
            started = perf_counter()
            infer(frames[i % len(frames)])
            samples.append((perf_counter() - started) * 1000)
        return samples

    with ThreadPoolExecutor(max_workers=streams) as pool:
        jobs = [pool.submit(run, infer) for infer in runners]
        started = perf_counter()
        barrier.wait()
        elapsed_samples = [sample for job in jobs for sample in job.result()]
        elapsed = perf_counter() - started
    return {"streams": streams, "frames": len(elapsed_samples), "fps": len(elapsed_samples) / elapsed,
            "p50_ms": float(np.median(elapsed_samples)), "p95_ms": float(np.percentile(elapsed_samples, 95))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runtime", choices=("cpu", "ncnn", "dawn"))
    parser.add_argument("model", type=Path, help="CPU reference model in ONNX format")
    parser.add_argument("frames", metavar="1280x720.bgr", nargs="+", type=Path)
    parser.add_argument("--ncnn-model", type=Path, help="converted base path ending in .ncnn")
    parser.add_argument("--ncnn-fp16", action="store_true", help="enable ncnn Vulkan FP16 storage and arithmetic")
    parser.add_argument("--adapter", type=int, default=0)
    parser.add_argument("--iterations", type=int, default=50)
    parser.add_argument("--allow-dawn-cpu-ops", action="store_true", help="measure a mixed GPU/CPU graph")
    parser.add_argument("--dawn-profile", type=Path, help="profile prefix for actual Dawn and CPU node counts")
    args = parser.parse_args()
    if args.iterations < 1 or args.adapter < 0:
        parser.error("iterations must be positive and adapter must be nonnegative")
    if args.runtime == "ncnn" and args.ncnn_model is None:
        parser.error("--ncnn-model is required for ncnn")
    frames = [input_tensor(path) for path in args.frames]
    cpu, _ = ort_factory(args.model, "cpu", args.adapter)
    reference = cpu()
    load, provider = (ncnn_factory(args.ncnn_model, args.adapter, args.ncnn_fp16) if args.runtime == "ncnn"
                      else ort_factory(args.model, args.runtime, args.adapter, args.allow_dawn_cpu_ops))
    if args.runtime == "dawn" and args.dawn_profile:
        infer, end_profile = load(args.dawn_profile)
    else:
        infer = load()
    comparisons = [agreement(reference(frame), infer(frame)) for frame in frames]
    if args.runtime == "dawn" and args.dawn_profile:
        entries = json.loads(Path(end_profile()).read_text(encoding="utf-8"))
        partition = Counter(entry["args"]["provider"] for entry in entries
                            if entry.get("cat") == "Node" and entry.get("args", {}).get("provider"))
        if not partition["WebGpuExecutionProvider"]:
            raise RuntimeError(f"Dawn executed no nodes: {partition}")
        print(json.dumps({"provider": provider, "node_executions": partition}), flush=True)
    print(json.dumps({"provider": provider, "comparisons": comparisons}), flush=True)
    del reference, infer
    for streams in (1, 4):
        print(json.dumps({"provider": provider, **measure(load, frames, streams, args.iterations)}), flush=True)


if __name__ == "__main__":
    main()
