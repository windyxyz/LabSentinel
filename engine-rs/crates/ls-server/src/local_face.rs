use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use image::{DynamicImage, ImageDecoder, ImageReader};
use ls_infer::face::{LocalFaceModel, EMBEDDING_SIZE};
use sha2::{Digest, Sha256};

use crate::face::{FaceMode, FaceSettings, LOCAL_PROFILE, MAX_PHOTO};
use crate::state::Shared;

const ZOO_REV: &str = "48b10fa505aeb51fbd127fc60a5c660040e823b7";
const MODELS: [Asset; 2] = [
    Asset {
        folder: "face_detection_yunet",
        filename: "face_detection_yunet_2023mar.onnx",
        sha256: "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4",
        size: 232_589,
    },
    Asset {
        folder: "face_recognition_sface",
        filename: "face_recognition_sface_2021dec.onnx",
        sha256: "0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79",
        size: 38_696_353,
    },
];

#[derive(Clone, Copy)]
struct Asset {
    folder: &'static str,
    filename: &'static str,
    sha256: &'static str,
    size: u64,
}

impl Asset {
    fn present(self, folder: &Path) -> bool {
        folder
            .join(self.filename)
            .metadata()
            .is_ok_and(|meta| meta.len() == self.size)
    }

    fn verify(self, path: &Path) -> Result<(), String> {
        let mut file = std::fs::File::open(path).map_err(|_| format!("缺少 {}", self.filename))?;
        let mut hash = Sha256::new();
        let bytes = std::io::copy(&mut file.by_ref().take(self.size + 1), &mut hash)
            .map_err(|_| "人脸模型读取失败")?;
        if bytes != self.size || format!("{:x}", hash.finalize()) != self.sha256 {
            return Err(format!("{} 的 SHA256 或文件大小不匹配", self.filename));
        }
        Ok(())
    }

    fn download(self, folder: &Path) -> Result<(), String> {
        let target = folder.join(self.filename);
        if self.verify(&target).is_ok() {
            return Ok(());
        }
        let url = format!(
            "https://media.githubusercontent.com/media/opencv/opencv_zoo/{ZOO_REV}/models/{}/{}",
            self.folder, self.filename
        );
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(120)))
            .max_redirects(0)
            .build()
            .into();
        let mut response = agent
            .get(&url)
            .call()
            .map_err(|_| format!("下载 {} 失败", self.filename))?;
        let temp = folder.join(format!(
            "{}.{}.part",
            self.filename,
            crate::envfile::random_token()
        ));
        let result = (|| {
            let mut file = std::fs::File::create(&temp).map_err(|_| "无法创建人脸模型文件")?;
            let mut body = response.body_mut().as_reader().take(self.size + 1);
            std::io::copy(&mut body, &mut file).map_err(|_| "下载人脸模型时连接中断")?;
            file.sync_all().map_err(|_| "无法写入人脸模型")?;
            drop(file);
            self.verify(&temp)?;
            std::fs::rename(&temp, &target)
                .map_err(|_| "无法保存人脸模型，请检查数据文件夹权限".to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
}

fn model_directory(data_dir: &str) -> PathBuf {
    Path::new(data_dir).join("models").join("face")
}

pub fn assets_present(data_dir: &str) -> bool {
    let directory = model_directory(data_dir);
    MODELS.iter().all(|model| model.present(&directory))
}

struct ModelCache {
    gpu: bool,
    adapter: Option<i32>,
    model: LocalFaceModel,
}

pub struct FaceRuntime {
    model: Mutex<Option<ModelCache>>,
    pub config_lock: Mutex<()>,
    pub library_lock: Mutex<()>,
    download: Mutex<()>,
    pub slots: Arc<tokio::sync::Semaphore>,
}

impl Default for FaceRuntime {
    fn default() -> Self {
        Self {
            model: Mutex::new(None),
            config_lock: Mutex::new(()),
            library_lock: Mutex::new(()),
            download: Mutex::new(()),
            slots: Arc::new(tokio::sync::Semaphore::new(2)),
        }
    }
}

impl FaceRuntime {
    pub fn install(&self, data_dir: &str) -> Result<(), String> {
        let _guard = self.download.try_lock().map_err(|_| "正在下载人脸模型")?;
        let folder = model_directory(data_dir);
        std::fs::create_dir_all(&folder).map_err(|_| "无法创建人脸模型目录")?;
        for model in MODELS {
            model.download(&folder)?;
        }
        Ok(())
    }

    pub fn devices(&self) -> Option<[bool; 2]> {
        let cache = self.model.try_lock().ok()?;
        cache.as_ref().map(|loaded| loaded.model.devices())
    }

    pub fn reset_model(&self) {
        if let Ok(mut cache) = self.model.try_lock() {
            cache.take();
        }
    }

    pub fn feature(
        &self,
        data_dir: &str,
        store: &ls_store::Store,
        photo: &Photo,
    ) -> Result<Vec<f32>, String> {
        let gpu = store.setting_get("infer.use_gpu").ok().flatten().as_deref() != Some("0")
            && crate::detect::runtime_ep_pref(store) == "dml";
        let adapter = gpu.then(ls_infer::ort_session::directml_adapter).flatten();
        let mut cache = self.model.lock().map_err(|_| "人脸模型暂不可用")?;
        if cache
            .as_ref()
            .is_none_or(|model| model.gpu != gpu || model.adapter != adapter)
        {
            let directory = model_directory(data_dir);
            let files = MODELS.map(|model| directory.join(model.filename));
            for (asset, file) in MODELS.into_iter().zip(&files) {
                asset.verify(file)?;
            }
            let model = LocalFaceModel::load(&files[0], &files[1], gpu)?;
            *cache = Some(ModelCache {
                gpu,
                adapter,
                model,
            });
        }
        cache
            .as_mut()
            .expect("model is loaded")
            .model
            .embedding(&photo.bgr, photo.w, photo.h)
    }
}

pub struct Photo {
    bgr: Vec<u8>,
    w: u32,
    h: u32,
}

impl Photo {
    pub fn decode(raw: &[u8], bounds: Option<[f32; 4]>) -> Result<Self, String> {
        Self::decode_cropped(raw, bounds, true)
    }

    pub(super) fn decode_person(raw: &[u8], bounds: Option<[f32; 4]>) -> Result<Self, String> {
        Self::decode_cropped(raw, bounds, false)
    }

    fn decode_cropped(
        raw: &[u8],
        bounds: Option<[f32; 4]>,
        face_only: bool,
    ) -> Result<Self, String> {
        if raw.is_empty() || raw.len() > MAX_PHOTO {
            return Err("请选择不超过 5 MB 的 JPEG 或 PNG 照片".into());
        }
        let mut reader = ImageReader::new(Cursor::new(raw))
            .with_guessed_format()
            .map_err(|_| "无法识别照片格式")?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let mut decoder = reader
            .into_decoder()
            .map_err(|_| "请使用 JPEG 或 PNG 格式，分辨率最大 4096×4096")?;
        let orientation = decoder
            .orientation()
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut image = DynamicImage::from_decoder(decoder).map_err(|_| "照片解码失败")?;
        image.apply_orientation(orientation);
        if let Some([x1, y1, x2, y2]) = bounds {
            let height = y2 - y1;
            let width = x2 - x1;
            let x_pad = if face_only { width * 0.12 } else { 0.0 };
            let y_pad = if face_only { height * 0.09 } else { 0.0 };
            let x = (x1 - x_pad).clamp(0.0, image.width() as f32) as u32;
            let y = (y1 - y_pad).clamp(0.0, image.height() as f32) as u32;
            let right = (x2 + x_pad).clamp(0.0, image.width() as f32) as u32;
            let bottom = (if face_only { y1 + height * 0.46 } else { y2 })
                .clamp(0.0, image.height() as f32) as u32;
            if right <= x || bottom <= y {
                return Err("人物区域无可用人脸画面".into());
            }
            image = image.crop_imm(x, y, right - x, bottom - y);
        }
        if image.width() > 1280 || image.height() > 1280 {
            image = image.thumbnail(1280, 1280);
        }
        let mut pixels = image.into_rgb8();
        let (w, h) = pixels.dimensions();
        for pixel in pixels.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        Ok(Self {
            bgr: pixels.into_raw(),
            w,
            h,
        })
    }

    pub fn jpeg(&self) -> Result<Vec<u8>, String> {
        ls_analyzer::evidence::encode_jpeg_bgr(&self.bgr, self.w, self.h)
    }
}

pub struct FaceMatch {
    pub id: i64,
    pub subject: String,
    pub similarity: f32,
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn recognize(
    st: &Shared,
    store: &ls_store::Store,
    cfg: &FaceSettings,
    photo: &Photo,
) -> Result<Option<FaceMatch>, String> {
    match cfg.active.mode {
        FaceMode::Local if cfg.local_enabled => {
            let feature = st.faces.feature(&st.data_dir, store, photo)?;
            let templates = store
                .face_templates(LOCAL_PROFILE, unix_now())
                .map_err(|_| "无法读取人脸模板")?;
            let fernet = crate::fernet::Fernet::load_or_create(Path::new(&st.data_dir))
                .ok_or("无法加载本机人脸模板密钥")?;
            let mut best = std::collections::HashMap::<i64, FaceMatch>::new();
            for template in templates {
                let Some(data) = template
                    .encrypted
                    .strip_prefix("enc:v1:")
                    .and_then(|cipher| fernet.decrypt(cipher))
                else {
                    continue;
                };
                let Ok(known) = serde_json::from_slice::<Vec<f32>>(&data) else {
                    continue;
                };
                if known.len() != EMBEDDING_SIZE {
                    continue;
                }
                let similarity = known.iter().zip(&feature).map(|(a, b)| a * b).sum::<f32>();
                if similarity.is_finite()
                    && best
                        .get(&template.person_id)
                        .is_none_or(|old| similarity > old.similarity)
                {
                    best.insert(
                        template.person_id,
                        FaceMatch {
                            id: template.person_id,
                            subject: template.subject,
                            similarity,
                        },
                    );
                }
            }
            let mut ranking: Vec<_> = best.into_values().collect();
            ranking.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
            let mut ranking = ranking.into_iter();
            let Some(candidate) = ranking.next() else {
                return Ok(None);
            };
            let ambiguous = ranking
                .next()
                .is_some_and(|next| candidate.similarity - next.similarity < 0.05);
            Ok((candidate.similarity >= cfg.local_threshold && !ambiguous).then_some(candidate))
        }
        FaceMode::Api => {
            let profile = cfg
                .api(&cfg.active.api)
                .filter(|p| p.enabled)
                .ok_or("请选择已启用的 API 模型")?;
            let client = profile.client(Path::new(&st.data_dir))?;
            let samples = client.recognize(&photo.jpeg()?)?;
            let Some((subject, similarity)) = samples.first() else {
                return Ok(None);
            };
            if *similarity < profile.threshold
                || samples
                    .iter()
                    .skip(1)
                    .any(|(other, value)| other != subject && similarity - value < 0.05)
            {
                return Ok(None);
            }
            let person = store
                .face_remote_match(&profile.id, subject, unix_now())
                .map_err(|_| "无法确认人员同意记录")?;
            Ok(person.map(|(id, subject)| FaceMatch {
                id,
                subject,
                similarity: *similarity,
            }))
        }
        _ => Err("人脸识别已关闭".into()),
    }
}

fn camera_allowed(store: &ls_store::Store, camera: i64, mode: FaceMode) -> bool {
    let Ok(Some(permission)) = store.setting_get(&format!("face.camera.{camera}")) else {
        return false;
    };
    match mode {
        FaceMode::Local => matches!(permission.as_str(), "local" | "local_or_api"),
        FaceMode::Api => permission == "local_or_api",
        FaceMode::Off => false,
    }
}

/// Use only the enabled local recognizer for matching review windows.
pub(super) fn review_subject(
    st: &Shared,
    store: &ls_store::Store,
    camera: i64,
    jpeg: &[u8],
    bounds: Option<[f32; 4]>,
) -> Option<FaceMatch> {
    if !camera_allowed(store, camera, FaceMode::Local) {
        return None;
    }
    let cfg = FaceSettings::load(store, Path::new(&st.data_dir));
    if cfg.active.mode != FaceMode::Local || cfg.active_profile().is_none() {
        return None;
    }
    let photo = Photo::decode(jpeg, bounds).ok()?;
    let person = recognize(st, store, &cfg, &photo).ok()??;
    (camera_allowed(store, camera, FaceMode::Local)
        && cfg.same_recognition(&FaceSettings::load(store, Path::new(&st.data_dir))))
    .then_some(person)
}

pub fn match_alert_enabled(store: &ls_store::Store, camera: i64) -> bool {
    store
        .setting_get(&format!("face.camera_alert.{camera}"))
        .is_ok_and(|value| value.as_deref() == Some("1"))
}

pub enum MatchTarget {
    Event(i64),
    Alert(i64),
}

pub fn identify_event(
    st: &Shared,
    store: &ls_store::Store,
    target: MatchTarget,
    camera: i64,
    bounds: [f32; 4],
    jpeg: &[u8],
) {
    let Ok(Some(permission)) = store.setting_get(&format!("face.camera.{camera}")) else {
        return;
    };
    if permission == "off" || jpeg.is_empty() {
        return;
    }
    let cfg = FaceSettings::load(store, Path::new(&st.data_dir));
    if cfg.active_profile().is_none() || !camera_allowed(store, camera, cfg.active.mode) {
        return;
    }
    let Ok(slot) = Arc::clone(&st.faces.slots).try_acquire_owned() else {
        return;
    };
    let st = Arc::clone(st);
    let data = jpeg.to_vec();
    let _ = std::thread::Builder::new()
        .name("face-event".into())
        .spawn(move || {
            let _slot = slot;
            let Ok(store) = ls_store::Store::open(&st.db_path) else {
                return;
            };
            let Ok(photo) = Photo::decode(&data, Some(bounds)) else {
                return;
            };
            let active = || {
                camera_allowed(&store, camera, cfg.active.mode)
                    && (!matches!(target, MatchTarget::Alert(_))
                        || match_alert_enabled(&store, camera))
                    && cfg.same_recognition(&FaceSettings::load(&store, Path::new(&st.data_dir)))
            };
            if !active() {
                return;
            }
            if let Ok(Some(person)) = recognize(&st, &store, &cfg, &photo) {
                if active() {
                    match target {
                        MatchTarget::Event(event) => {
                            let _ = store.save_face_match(
                                event,
                                person.id,
                                person.similarity,
                                unix_now(),
                            );
                        }
                        MatchTarget::Alert(track) => {
                            publish_match_alert(&st, &store, camera, track, &person);
                        }
                    }
                }
            }
        });
}

fn publish_match_alert(
    st: &Shared,
    store: &ls_store::Store,
    camera: i64,
    track: i64,
    person: &FaceMatch,
) {
    let now = SystemTime::now();
    let ts = ls_core::time::format_system_time_local_iso(now);
    let since = ls_core::time::format_system_time_local_iso(now - Duration::from_secs(90));
    let event = ls_analyzer::pipeline::Candidate {
        camera_id: camera,
        zone_id: None,
        behavior: "face_match".into(),
        level: 2,
        track_id: track,
        detail: "检测到与已录入人员匹配的人脸，请核对事件详情".into(),
        ts_iso: ts,
        evidence_key: String::new(),
    };
    let row = ls_store::NewEvent {
        camera_id: camera,
        type_: &event.behavior,
        level: event.level,
        track_id: track,
        detail: &event.detail,
        snapshot: None,
        ts: &event.ts_iso,
    };
    if let Ok(Some(id)) =
        store.add_face_alert(&row, person.id, person.similarity, unix_now(), &since)
    {
        if let Ok(Some(row)) = store.event_by_id(id) {
            let _ = st
                .events_tx
                .send(crate::handlers::events::event_json(&row).to_string());
        }
        crate::worker::EventSink::new(Arc::clone(st)).deliver_deferred_pub(id, &event, false);
    }
}

pub fn start_retention(st: Shared) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15 * 60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let st = Arc::clone(&st);
            let _ = tokio::task::spawn_blocking(move || {
                let Ok(_guard) = st.faces.library_lock.try_lock() else {
                    return;
                };
                let Ok(store) = ls_store::Store::open(&st.db_path) else {
                    return;
                };
                let Ok(people) = store.face_people() else {
                    return;
                };
                let cfg = FaceSettings::load(&store, Path::new(&st.data_dir));
                let now = unix_now();
                for person in people
                    .iter()
                    .filter(|person| person.removing || person.expires_at <= now)
                {
                    let _ = crate::handlers::faces::remove_person(&st, &store, &cfg, person);
                }
                for (profile, remote) in store.face_remote_removals().unwrap_or_default() {
                    if let Some(client) = cfg
                        .api(&profile)
                        .and_then(|p| p.client(Path::new(&st.data_dir)).ok())
                    {
                        if client.delete_person(&remote).is_ok() {
                            let _ = store.clear_face_remote_removal(&profile, &remote);
                        }
                    }
                }
                let _ = store.prune_face_matches();
            })
            .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the pinned official ONNX files and permitted face test images"]
    fn local_face_model_matches_reference_pairs() {
        let dir = PathBuf::from(
            std::env::var_os("LS_FACE_TEST_ASSETS").expect("设定 LS_FACE_TEST_ASSETS"),
        );
        let model_path = MODELS.map(|a| dir.join(a.filename));
        for (asset, path) in MODELS.into_iter().zip(&model_path) {
            asset.verify(path).unwrap();
        }
        let gpu = std::env::var("LS_FACE_TEST_DEVICE").as_deref() == Ok("directml");
        let mut model = LocalFaceModel::load(&model_path[0], &model_path[1], gpu).unwrap();
        if gpu {
            assert_eq!(
                model.devices(),
                [true, true],
                "DirectML 被请求但模型回落了 CPU"
            );
        }
        let features = [0, 1, 2].map(|i| {
            let data = std::fs::read(dir.join(format!("sample-{i}.jpg"))).unwrap();
            let photo = Photo::decode(&data, None).unwrap();
            let start = std::time::Instant::now();
            let embedding = model.embedding(&photo.bgr, photo.w, photo.h).unwrap();
            println!(
                "sample={i} latency_ms={:.2}",
                start.elapsed().as_secs_f64() * 1000.0
            );
            embedding
        });
        let score = |i: usize| {
            features[0]
                .iter()
                .zip(&features[i])
                .map(|(a, b)| a * b)
                .sum::<f32>()
        };
        println!(
            "matched={:.4} distinct={:.4} devices={:?}",
            score(1),
            score(2),
            model.devices()
        );
        assert!(score(1) >= 0.5);
        assert!(score(2) < 0.5);
    }
}
