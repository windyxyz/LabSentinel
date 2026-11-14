//! 生成 proto/analyzer.proto 的 tonic/prost 代码（T-320 契约唯一实现源）。
//! protoc 用 protoc-bin-vendored（本机无系统 protoc）。
//! 注意：protoc（C++ 工具）对中文仓路径按错误代码页解析会 "directory does not exist"——
//! 先把 proto 拷到 ASCII 旁路目录再编译，生成结果与源路径无关。旁路目录用 OUT_DIR
//! （target 目录通常为 ASCII；CI 亦然），仅当 OUT_DIR 含非 ASCII 时退回系统临时目录。

use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let proto_file = proto_root.join("proto/analyzer.proto");
    println!("cargo:rerun-if-changed={}", proto_file.display());

    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let work = if out_dir.to_string_lossy().is_ascii() {
        out_dir.join("ls-proto-build")
    } else {
        std::env::temp_dir().join("ls-proto-build")
    };
    fs::create_dir_all(&work)?;
    let ascii_proto = work.join("analyzer.proto");
    fs::copy(&proto_file, &ascii_proto)?;

    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);
    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .compile_protos(&[ascii_proto], &[work])?;
    Ok(())
}
