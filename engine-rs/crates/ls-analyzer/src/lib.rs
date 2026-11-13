//! ls-analyzer 库目标（ADR-0011 / RA1.0）：把纯数据面模块从 bin 中拆出，供
//! ls-server 进程内复用（管线/存证环零逻辑改动，gRPC 服务本体仍在 main.rs）。
//!
//! 拆分纪律（RA1 首切）：pipeline/evidence 保持纯函数式、不引 tokio 运行时
//! 依赖，事件出口由调用方以回调注入（见 ls-server worker.rs RA1.1）。

pub mod detector;
pub mod evidence;
pub mod pipeline;
pub mod ppe;
