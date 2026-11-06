//! proto/analyzer.proto 生成代码（tonic/prost）。
// tonic-build 产物有大量风格类告警——对生成代码统一豁免（手写 crate 仍 -D warnings）
#![allow(clippy::all, clippy::pedantic, dead_code, unused_imports)]

pub mod analyzer {
    tonic::include_proto!("labsentinel.analyzer.v1");
}
