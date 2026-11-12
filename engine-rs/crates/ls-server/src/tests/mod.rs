// A4（2026-11-12）：ls-server 测试按域拆分，helper 集中。
pub(crate) mod helpers; // worker.rs（RA1.2）等业务模块的测试复用 test_state_custom
mod routes_guard;
mod stats_export;
