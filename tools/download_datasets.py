# -*- coding: utf-8 -*-
"""下载 LAB PPE 数据集（Roboflow Universe，CC BY 4.0）到 datasets/lab-ppe/。

来源: https://universe.roboflow.com/gavriels-workspace-e9azv/lab-ppe-p4qe4/dataset/1
（10 类，588 训练 / 40 验证 / 40 测试图，YOLO 格式）

用法（m0-venv）:
    # 1) 设置 Roboflow 私人 API key（免费注册 https://app.roboflow.com -> Settings -> API Key）
    #    Windows CMD:  set RF_API_KEY=<你的Key>
    #    Git Bash:     export RF_API_KEY=<你的Key>
    # 2) 本机直连 Roboflow 会失败，需走本地代理（roboflow 包遵循 HTTPS_PROXY 环境变量）
    HTTPS_PROXY=http://127.0.0.1:7890 D:\\LabSentinel\\m0-venv\\Scripts\\python.exe tools/download_datasets.py

key 从环境变量 RF_API_KEY 读取，禁止写死到代码/命令行历史；数据集已存在则跳过。
"""
import argparse
import os
import sys
from pathlib import Path

# 项目根目录（脚本在 tools/ 下，无论从哪里调用路径都成立）
ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT = ROOT / "datasets" / "lab-ppe"


def parse_args():
    ap = argparse.ArgumentParser(
        description="从 Roboflow Universe 下载 LAB PPE 数据集（key 从环境变量 RF_API_KEY 读取）"
    )
    ap.add_argument("--workspace", default="gavriels-workspace-e9azv",
                    help="Roboflow workspace 名（默认 gavriels-workspace-e9azv）")
    ap.add_argument("--project", default="lab-ppe-p4qe4",
                    help="Roboflow project 名（默认 lab-ppe-p4qe4）")
    ap.add_argument("--version", type=int, default=1,
                    help="数据集版本号（默认 1）")
    ap.add_argument("--format", default="yolov8",
                    help="导出格式，默认 yolov8（YOLO 目录布局，与现有 data.yaml 一致）")
    ap.add_argument("--out", default=str(DEFAULT_OUT),
                    help=f"下载目标目录（默认 {DEFAULT_OUT}）")
    return ap.parse_args()


def main():
    args = parse_args()
    out = Path(args.out)

    # 已存在则跳过（以 data.yaml 为存在标志）
    if (out / "data.yaml").exists():
        print(f"[skip] {out} 已存在 data.yaml，视为数据集已下载，跳过。")
        print(f"       如需强制重下，请先删除该目录或用 --out 指定新目录。")
        return

    # key 必须来自环境变量，缺失时给出清晰设置说明
    api_key = os.environ.get("RF_API_KEY", "").strip()
    if not api_key:
        sys.exit(
            "[error] 未设置环境变量 RF_API_KEY（Roboflow 私人 API key）。\n"
            "        获取方式: 注册 https://app.roboflow.com（免费）-> Settings -> API Key -> Copy。\n"
            "        设置方式:\n"
            "          Git Bash :  export RF_API_KEY=<你的Key>\n"
            "          CMD      :  set RF_API_KEY=<你的Key>\n"
            "          PowerShell: $env:RF_API_KEY='<你的Key>'"
        )

    # 代理提示：roboflow 包基于 requests，自动遵循 HTTPS_PROXY/HTTP_PROXY 环境变量
    proxy = os.environ.get("HTTPS_PROXY") or os.environ.get("https_proxy")
    if proxy:
        print(f"[info] 检测到代理 HTTPS_PROXY={proxy}（Roboflow 需要走代理）")
    else:
        print("[warn] 未检测到 HTTPS_PROXY。本机直连 Roboflow 会失败，"
              "建议先执行: export HTTPS_PROXY=http://127.0.0.1:7890")

    # 延迟导入：缺 key / --help 时不需要 roboflow 包
    from roboflow import Roboflow

    print(f"[info] 下载 {args.workspace}/{args.project} v{args.version} (format={args.format}) -> {out}")
    rf = Roboflow(api_key=api_key)
    project = rf.workspace(args.workspace).project(args.project)
    version = project.version(args.version)
    version.download(args.format, location=str(out))
    print(f"[done] 数据集已下载到 {out}")
    print(f"[next] 检查 {out / 'data.yaml'} 的 names 列表（类别顺序，后续训练/导出依赖它）")


if __name__ == "__main__":
    main()
