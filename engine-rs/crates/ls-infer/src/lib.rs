//! ls-infer：推理层（T-325）。
//! - [`post`]：YOLO 后处理（letterbox 反变换/解码/NMS，纯 Rust，与 Python 逐位对齐）
//! - [`yolo`]：tract 纯 Rust ONNX 推理（feature=tract；无需 libclang/ORT 运行时）
//!
//! 说明：76 文档原定 ort；本机 windows-gnu 目标缺 libclang，ort-sys 无法生成绑定。
//! 采用 tract（纯 Rust）先打通真实检测→事件闭环，ort 后端保留为后续可替换实现。

pub mod post;

#[cfg(feature = "tract")]
pub mod yolo;
