//! ls-analyzer sidecar（T-327）：监听 127.0.0.1:<port>，一次性认证令牌经启动参数传入
//! （--token <secret>，Python 父进程生成），防本机其它进程未授权调用。
//!
//! 数据面（ingest/infer/track）当前为骨架：状态机与事件流通道已就绪，
//! 检测数据源由 T-325（ls-infer/ort）与 T-326（ls-ingest）填充。
//! 启动即打印 "READY port=<p>" 行，供父进程健康判定。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use ls_proto::analyzer::analyzer_server::{Analyzer, AnalyzerServer};
use ls_proto::analyzer::{
    Ack, AnalyzerStatus, CameraRef, CameraStatus, Config, Empty, EventCandidate, EvidenceRequest,
    EvidenceResult, EventsRequest, Jpeg,
};

#[derive(Clone)]
struct CamState {
    state: &'static str, // running | stopped | error
    fps_now: f32,
}

struct AppStateInner {
    token: String,
    config_version: i64,
    cams: HashMap<i64, CamState>,
    /// 每个事件流订阅者的发送端（StartCamera/Stop 时广播状态变化占位——真实事件由数据面填）
    event_txs: Vec<mpsc::Sender<Result<EventCandidate, Status>>>,
}

type AppState = Arc<Mutex<AppStateInner>>;

pub struct AnalyzerImpl {
    state: AppState,
}

impl AnalyzerImpl {
    fn new(token: String) -> Self {
        AnalyzerImpl {
            state: Arc::new(Mutex::new(AppStateInner {
                token,
                config_version: 0,
                cams: HashMap::new(),
                event_txs: vec![],
            })),
        }
    }

    fn check_auth<T>(&self, req: &Request<T>) -> Result<(), Status> {
        let got = req
            .metadata()
            .get("x-ls-token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let want = self.state.lock().unwrap().token.clone();
        if want.is_empty() || constant_eq(got, &want) {
            Ok(())
        } else {
            Err(Status::unauthenticated("invalid token"))
        }
    }
}

/// 常量时间比较（防时序侧信道）。
fn constant_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[tonic::async_trait]
impl Analyzer for AnalyzerImpl {
    async fn apply_config(&self, request: Request<Config>) -> Result<Response<Ack>, Status> {
        self.check_auth(&request)?;
        let cfg = request.into_inner();
        let mut st = self.state.lock().unwrap();
        st.config_version = cfg.version;
        Ok(Response::new(Ack { ok: true, message: format!("version={}", cfg.version) }))
    }

    async fn start_camera(&self, request: Request<CameraRef>) -> Result<Response<Ack>, Status> {
        self.check_auth(&request)?;
        let cid = request.into_inner().camera_id;
        let mut st = self.state.lock().unwrap();
        st.cams.insert(
            cid,
            CamState { state: "running", fps_now: 0.0 },
        );
        // 数据面接入后此处 spawn 摄像头任务；事件经 event_txs 广播
        Ok(Response::new(Ack { ok: true, message: String::new() }))
    }

    async fn stop_camera(&self, request: Request<CameraRef>) -> Result<Response<Ack>, Status> {
        self.check_auth(&request)?;
        let cid = request.into_inner().camera_id;
        let mut st = self.state.lock().unwrap();
        st.cams.insert(
            cid,
            CamState { state: "stopped", fps_now: 0.0 },
        );
        Ok(Response::new(Ack { ok: true, message: String::new() }))
    }

    type EventsStream = tokio_stream::wrappers::ReceiverStream<Result<EventCandidate, Status>>;

    async fn events(
        &self,
        request: Request<EventsRequest>,
    ) -> Result<Response<Self::EventsStream>, Status> {
        self.check_auth(&request)?;
        let (tx, rx) = mpsc::channel(64);
        self.state.lock().unwrap().event_txs.push(tx);
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn status(&self, request: Request<Empty>) -> Result<Response<AnalyzerStatus>, Status> {
        self.check_auth(&request)?;
        let st = self.state.lock().unwrap();
        let mut cams: Vec<CameraStatus> = st
            .cams
            .iter()
            .map(|(cid, s)| CameraStatus {
                camera_id: *cid,
                state: s.state.to_string(),
                analysis_fps_now: s.fps_now,
                frame_age_s: 0.0,
                reconnect_count: 0,
                last_error: String::new(),
                infer_ms_p50: 0.0,
                queue_depth: 0,
            })
            .collect();
        cams.sort_by_key(|c| c.camera_id);
        Ok(Response::new(AnalyzerStatus { cameras: cams }))
    }

    async fn latest_frame(&self, request: Request<CameraRef>) -> Result<Response<Jpeg>, Status> {
        self.check_auth(&request)?;
        let _cid = request.into_inner().camera_id;
        // T-326 数据面填充；骨架期返回空 JPEG
        Ok(Response::new(Jpeg { data: vec![] }))
    }

    async fn finalize_evidence(
        &self,
        request: Request<EvidenceRequest>,
    ) -> Result<Response<EvidenceResult>, Status> {
        self.check_auth(&request)?;
        let _req = request.into_inner();
        // T-326 存证 remux 填充；骨架期返回 not-implemented 语义（ok=false）
        Ok(Response::new(EvidenceResult {
            ok: false,
            video_relpath: String::new(),
            message: "evidence remux not implemented until T-326".to_string(),
        }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut port: u16 = 59001;
    let mut token = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().ok_or("--port 缺值")?.parse()?,
            "--token" => token = args.next().ok_or("--token 缺值")?,
            other => return Err(format!("未知参数 {other}（--port/--token）").into()),
        }
    }
    if token.is_empty() {
        return Err("必须提供 --token（一次性认证令牌，由父进程生成）".into());
    }
    let addr = format!("127.0.0.1:{port}").parse()?;
    let svc = AnalyzerServer::new(AnalyzerImpl::new(token));
    println!("READY port={port}");
    Server::builder()
        .add_service(svc)
        .serve(addr)
        .await?;
    Ok(())
}
