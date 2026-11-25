use ort::session::Session;
use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::SystemTime;

#[derive(Clone, Copy, Default)]
pub enum GpuPriority {
    Live,
    #[default]
    Normal,
    Background,
}

thread_local! {
    static PRIORITY: Cell<GpuPriority> = const { Cell::new(GpuPriority::Normal) };
}

/// Sets the priority for the analysis thread. Workers processing a live stream use Live.
pub fn set_thread_gpu_priority(priority: GpuPriority) {
    PRIORITY.set(priority);
}

type Job = Box<dyn FnOnce(&mut Sessions) + Send>;

#[derive(Default)]
struct Queue {
    lanes: [VecDeque<Job>; 3],
    turn: usize,
}

impl Queue {
    fn len(&self) -> usize {
        self.lanes.iter().map(VecDeque::len).sum()
    }

    fn next(&mut self) -> Option<Job> {
        const ORDER: [usize; 12] = [0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 2];
        for _ in ORDER {
            let lane = ORDER[self.turn];
            self.turn = (self.turn + 1) % ORDER.len();
            if let Some(job) = self.lanes[lane].pop_front() {
                return Some(job);
            }
        }
        None
    }
}

#[derive(Default)]
struct GpuScheduler {
    queue: Mutex<Queue>,
    ready: Condvar,
    space: Condvar,
}

impl GpuScheduler {
    fn process() -> Result<&'static Self, String> {
        static SCHEDULER: OnceLock<Result<Arc<GpuScheduler>, String>> = OnceLock::new();
        SCHEDULER
            .get_or_init(|| {
                let scheduler = Arc::new(Self::default());
                let worker = Arc::clone(&scheduler);
                std::thread::Builder::new()
                    .name("ls-dml-worker".into())
                    .spawn(move || worker.run())
                    .map_err(|error| format!("无法启动 DirectML 工作线程: {error}"))?;
                Ok(scheduler)
            })
            .as_deref()
            .map_err(Clone::clone)
    }

    fn run(&self) {
        let mut sessions = Sessions::default();
        loop {
            let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
            let job = loop {
                if let Some(job) = queue.next() {
                    break job;
                }
                queue = self
                    .ready
                    .wait(queue)
                    .unwrap_or_else(|error| error.into_inner());
            };
            self.space.notify_one();
            drop(queue);
            let _guard = dml_lock().lock().unwrap_or_else(|error| error.into_inner());
            // A failed request can release its session without terminating the DirectML worker.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&mut sessions)));
        }
    }

    fn execute<T: Send + 'static>(
        &self,
        priority: GpuPriority,
        action: impl FnOnce(&mut Sessions) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        const LIMIT: usize = 64;
        let (send, receive) = mpsc::sync_channel(1);
        let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        while queue.len() >= LIMIT {
            queue = self
                .space
                .wait(queue)
                .unwrap_or_else(|error| error.into_inner());
        }
        queue.lanes[priority as usize].push_back(Box::new(move |sessions| {
            let _ = send.send(action(sessions));
        }));
        self.ready.notify_one();
        drop(queue);
        receive
            .recv()
            .map_err(|_| "DirectML 工作项异常结束".to_string())?
    }

    fn release(&self, id: u64) {
        let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
        // A lease may be dropped by a job on this thread. Cleanup must never wait for queue space.
        queue.lanes[GpuPriority::Normal as usize].push_back(Box::new(move |state| {
            state.sessions.remove(&id);
            state.models.retain(|_, lease| lease.strong_count() > 0);
        }));
        self.ready.notify_one();
    }
}

#[derive(Default)]
struct Sessions {
    sessions: HashMap<u64, Session>,
    models: HashMap<ModelKey, Weak<GpuLease>>,
    last_id: u64,
}

#[derive(Eq, Hash, PartialEq)]
struct ModelKey {
    path: PathBuf,
    modified: Option<SystemTime>,
    length: u64,
    adapter: Option<i32>,
}

impl ModelKey {
    fn new(path: &Path, adapter: Option<i32>) -> Result<Self, String> {
        let path = path
            .canonicalize()
            .map_err(|error| format!("找不到模型: {error}"))?;
        let stat = path
            .metadata()
            .map_err(|error| format!("读取模型属性失败: {error}"))?;
        Ok(Self {
            path,
            modified: stat.modified().ok(),
            length: stat.len(),
            adapter,
        })
    }
}

pub(crate) struct GpuLease(u64);

impl Drop for GpuLease {
    fn drop(&mut self) {
        if let Ok(worker) = GpuScheduler::process() {
            worker.release(self.0);
        }
    }
}

pub(crate) enum OrtSession {
    Cpu(Box<Session>),
    Gpu(Arc<GpuLease>),
}

impl OrtSession {
    pub(crate) fn load(path: &Path, use_gpu: bool) -> Result<Self, String> {
        if use_gpu {
            let key = ModelKey::new(path, directml_adapter())?;
            let result = GpuScheduler::process().and_then(|worker| {
                worker.execute(PRIORITY.get(), move |state| {
                    if let Some(lease) = state.models.get(&key).and_then(Weak::upgrade) {
                        return Ok(lease);
                    }
                    let session = build(&key.path, Some(key.adapter), None)?;
                    state.last_id += 1;
                    let lease = Arc::new(GpuLease(state.last_id));
                    state.sessions.insert(lease.0, session);
                    state.models.insert(key, Arc::downgrade(&lease));
                    Ok(lease)
                })
            });
            match result {
                Ok(lease) => return Ok(Self::Gpu(lease)),
                Err(error) => {
                    eprintln!("[ls-infer] GPU(DirectML) 初始化失败，自动回落 CPU: {error}")
                }
            }
        }
        build(path, None, None).map(Box::new).map(Self::Cpu)
    }

    pub(crate) fn load_background_cpu(path: &Path) -> Result<Self, String> {
        build(path, None, Some(1)).map(Box::new).map(Self::Cpu)
    }

    pub(crate) fn is_gpu(&self) -> bool {
        matches!(self, Self::Gpu(_))
    }

    pub(crate) fn with<T: Send + 'static>(
        &mut self,
        priority: Option<GpuPriority>,
        action: impl FnOnce(&mut Session) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        match self {
            Self::Cpu(session) => action(session),
            Self::Gpu(lease) => {
                let lease = Arc::clone(lease);
                GpuScheduler::process()?.execute(
                    priority.unwrap_or_else(|| PRIORITY.get()),
                    move |state| {
                        let session = state
                            .sessions
                            .get_mut(&lease.0)
                            .ok_or("DirectML 会话已释放")?;
                        action(session)
                    },
                )
            }
        }
    }
}

/// The adapter is the DXGI index, independent of the Vulkan enumeration order.
static ADAPTER: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

pub fn select_directml_adapter(adapter: Option<i32>) {
    ADAPTER.store(adapter.unwrap_or(-1), std::sync::atomic::Ordering::Relaxed);
}

pub fn directml_adapter() -> Option<i32> {
    let value = ADAPTER.load(std::sync::atomic::Ordering::Relaxed);
    (value >= 0).then_some(value)
}

pub fn dml_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}

fn build(path: &Path, dml: Option<Option<i32>>, threads: Option<usize>) -> Result<Session, String> {
    static ENVIRONMENT: OnceLock<Result<ort::environment::Environment, String>> = OnceLock::new();
    let environment = ENVIRONMENT
        .get_or_init(|| {
            ort::environment::init()
                .with_name("labsentinel")
                .build()
                .map_err(|error| format!("ORT 环境初始化失败: {error}（请检查 ORT_DYLIB_PATH）"))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let bytes = std::fs::read(path).map_err(|error| format!("读取模型失败: {error}"))?;
    let mut builder = Session::builder(environment).map_err(|error| error.to_string())?;
    if let Some(threads) = threads {
        builder = builder
            .with_intra_threads(threads)
            .map_err(|error| error.to_string())?
            .with_parallel_execution(false)
            .map_err(|error| error.to_string())?;
    }
    if let Some(adapter) = dml {
        let mut provider = ort::ep::DirectML::default();
        if let Some(adapter) = adapter {
            provider = provider.with_device_id(adapter);
        }
        builder = builder
            .with_memory_pattern(false)
            .map_err(|error| error.to_string())?
            .with_parallel_execution(false)
            .map_err(|error| error.to_string())?
            .with_execution_providers([provider.build().error_on_failure()])
            .map_err(|error| error.to_string())?;
    }
    builder
        .commit_from_memory(&bytes)
        .map_err(|error| format!("ORT 加载模型失败: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_uses_one_named_thread_for_all_priorities() {
        let ids: Vec<_> = (0..6)
            .map(|index| {
                std::thread::spawn(move || {
                    let priority = [
                        GpuPriority::Live,
                        GpuPriority::Normal,
                        GpuPriority::Background,
                    ][index % 3];
                    GpuScheduler::process()
                        .unwrap()
                        .execute(priority, |_| {
                            assert_eq!(std::thread::current().name(), Some("ls-dml-worker"));
                            Ok(std::thread::current().id())
                        })
                        .unwrap()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert!(ids.iter().all(|id| *id == ids[0]));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires a physical DirectML adapter and the packaged ONNX Runtime"]
    fn two_camera_leases_share_the_directml_session() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Resources/dist/rust/labsentinel-server/models/yolo11n.onnx");
        let mut first = OrtSession::load(&path, true).unwrap();
        let second = OrtSession::load(&path, true).unwrap();
        let (OrtSession::Gpu(a), OrtSession::Gpu(b)) = (&first, &second) else {
            panic!("the model fell back to CPU");
        };
        assert!(Arc::ptr_eq(a, b));
        drop(second);
        first
            .with(None, |session| {
                let input =
                    ort::value::Tensor::from_array(([1, 3, 640, 640], vec![0f32; 3 * 640 * 640]))
                        .map_err(|error| error.to_string())?;
                let output = session
                    .run(ort::inputs![input])
                    .map_err(|error| error.to_string())?;
                assert!(output.iter().next().is_some());
                Ok(())
            })
            .unwrap();
    }
}
