//! Original encoded video buffered in memory for event-triggered clips.
//!
//! A recorder belongs to one FFmpeg session. No bytes are sent to disk until
//! a caller triggers and writes a clip. The decoded analysis stream remains
//! usable when a camera, codec or fragment cannot be recorded.

mod mp4;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use mp4::{Fragment, Init, TIMESCALE};

const BUFFER_TIME: Duration = Duration::from_secs(12);
const BUFFER_BYTES: usize = 16 * 1024 * 1024;
const MAX_FRAGMENTS: usize = 1024;
const MAX_PENDING_CLIPS: usize = 3;
const MAX_GAP_TICKS: i64 = 3 * TIMESCALE;
const MAX_INPUT_LAG: Duration = Duration::from_secs(5);
const MAX_BATCH_LEAD: Duration = Duration::from_millis(200);
const MAX_WALL_CLOCK_JUMP: Duration = Duration::from_secs(1);
const PIPE_BUFFER: usize = 64 * 1024;

/// The second FFmpeg output copies the same input stream to stderr with its
/// diagnostic output disabled. A null tee output keeps decoding alive if MP4
/// does not support the camera's codec or the recording pipe closes.
pub(crate) fn add_ffmpeg_output(cmd: &mut Command) {
    // The tee muxer otherwise inherits input-specific codec tags (e.g. MPEG-TS
    // stream type 0x1b). Only the MP4 slave with the matching tag writes to
    // the pipe. Both mappings reference the same encoded packets.
    const MP4: &str = "f=mp4:movflags=+delay_moov+empty_moov+default_base_moof+frag_every_frame+skip_trailer:video_track_timescale=90000:use_editlist=0:flush_packets=1";
    cmd.args([
        "-map",
        "0:v:0",
        "-map",
        "0:v:0",
        "-c:v",
        "copy",
        "-tag:v:0",
        "avc1",
        "-tag:v:1",
        "hvc1",
        "-map_metadata",
        "-1",
        "-map_metadata:s:v",
        "-1",
        "-map_chapters",
        "-1",
        "-f",
        "tee",
    ])
    .arg(format!(
        "[onfail=ignore:select=0:{MP4}]pipe:2|[onfail=ignore:select=1:{MP4}]pipe:2|[f=null]-"
    ));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    Hevc,
}

impl VideoCodec {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc => "hevc",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingError {
    Unavailable,
    UnsupportedCodec,
    InvalidStream,
    InvalidWindow,
    NoKeyframe,
    BufferLimit,
    TimestampDiscontinuity,
    SourceEnded,
    TimedOut,
    Busy,
}

impl RecordingError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::UnsupportedCodec => "unsupported_codec",
            Self::InvalidStream => "invalid_stream",
            Self::InvalidWindow => "invalid_window",
            Self::NoKeyframe => "no_keyframe",
            Self::BufferLimit => "buffer_limit",
            Self::TimestampDiscontinuity => "timestamp_discontinuity",
            Self::SourceEnded => "source_ended",
            Self::TimedOut => "timed_out",
            Self::Busy => "busy",
        }
    }
}

impl fmt::Display for RecordingError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(self.as_str())
    }
}

impl std::error::Error for RecordingError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipCoverage {
    /// Source presentation timestamps anchored to arrival on the server clock.
    pub start: SystemTime,
    pub end: SystemTime,
    pub codec: VideoCodec,
    pub truncation: Option<RecordingError>,
    pub keyframes: Vec<KeyframeIndex>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyframeIndex {
    pub timestamp: SystemTime,
    /// Start of the MP4 fragment containing the keyframe.
    pub offset: u64,
}

/// Cloneable handle for the current input session. Take a new handle on reconnect.
#[derive(Clone)]
pub struct Recorder(Arc<Session>);

struct Session {
    state: Mutex<State>,
    ready: Condvar,
    bytes: Arc<AtomicUsize>,
    active: AtomicUsize,
}

impl Recorder {
    fn new() -> Self {
        Self(Arc::new(Session {
            state: Mutex::new(State::default()),
            ready: Condvar::new(),
            bytes: Arc::default(),
            active: AtomicUsize::new(0),
        }))
    }

    pub(crate) fn start(pipe: impl Read + Send + 'static) -> io::Result<(Self, JoinHandle<()>)> {
        let recorder = Self::new();
        let producer = recorder.clone();
        let thread = thread::Builder::new()
            .name("encoded-capture".into())
            .spawn(move || {
                let mut input = BufReader::with_capacity(PIPE_BUFFER, pipe);
                let reason = mp4::read_stream(&mut input, &producer)
                    .err()
                    .unwrap_or(RecordingError::SourceEnded);
                producer.finish(reason);
                // Keep the decoder moving if the unsupported or broken tap still writes.
                let _ = io::copy(&mut input, &mut io::sink());
            })?;
        Ok((recorder, thread))
    }

    /// Reserve an event clip. Waiting and file I/O belong on a background thread.
    pub fn trigger(
        &self,
        event_at: SystemTime,
        pre: Duration,
        post: Duration,
    ) -> Result<PendingClip, RecordingError> {
        let window = Window {
            start: event_at
                .checked_sub(pre)
                .ok_or(RecordingError::InvalidWindow)?,
            event: event_at,
            end: event_at
                .checked_add(post)
                .ok_or(RecordingError::InvalidWindow)?,
        };
        let state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        let first = state
            .groups
            .front()
            .and_then(|group| state.presentation_time(&group[0]));
        if first.ok_or_else(|| state.finished.unwrap_or(state.gap))? > event_at {
            return Err(state.gap);
        }
        reserve(&self.0.active, 1, MAX_PENDING_CLIPS).ok_or(RecordingError::Busy)?;
        Ok(PendingClip {
            window,
            epoch: state.epoch,
            permit: ClipPermit(Arc::clone(&self.0)),
        })
    }

    fn set_init(&self, init: Arc<Init>) {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner()).init = Some(init);
    }

    fn feed(&self, fragment: Fragment) {
        let received = Arrival {
            wall: SystemTime::now(),
            monotonic: Instant::now(),
        };
        self.0.state.lock().unwrap_or_else(|e| e.into_inner()).push(
            fragment,
            &self.0.bytes,
            received,
        );
        if self.0.active.load(Ordering::Relaxed) > 0 {
            self.0.ready.notify_all();
        }
    }

    fn discontinuity(&self, reason: RecordingError) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reset(reason);
        self.0.ready.notify_all();
    }

    fn finish(&self, reason: RecordingError) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .finished = Some(reason);
        self.0.ready.notify_all();
    }
}

#[derive(Clone, Copy)]
struct Window {
    start: SystemTime,
    event: SystemTime,
    end: SystemTime,
}

pub struct PendingClip {
    window: Window,
    epoch: u64,
    permit: ClipPermit,
}

impl PendingClip {
    /// Wait for the post-event samples. A bounded, playable partial result is
    /// returned if it still covers the event when capture stops or times out.
    pub fn wait(self, timeout: Duration) -> Result<Clip, RecordingError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(RecordingError::InvalidWindow)?;
        let session = &self.permit.0;
        let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if self.epoch != state.epoch {
                return Err(state.invalidated_by);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let stopped = state
                .finished
                .or_else(|| remaining.is_zero().then_some(RecordingError::TimedOut));
            if stopped.is_some() || state.reached(self.window.end) {
                let selected = state.select(self.window, stopped)?;
                drop(state);
                return Ok(Clip {
                    selected,
                    _permit: self.permit,
                });
            }
            (state, _) = session
                .ready
                .wait_timeout(state, remaining)
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}

struct ClipPermit(Arc<Session>);

impl Drop for ClipPermit {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

pub struct Clip {
    selected: Selection,
    _permit: ClipPermit,
}

impl Clip {
    /// Publish a complete fMP4 by renaming a temporary file in the same directory.
    /// The caller supplies the destination directory and registers the path after success.
    pub fn write_mp4(mut self, out: &Path) -> io::Result<ClipCoverage> {
        static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
        let mut name = out
            .file_name()
            .ok_or(io::ErrorKind::InvalidInput)?
            .to_os_string();
        let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        name.push(format!(".{}-{sequence}.part", std::process::id()));
        let temp = out.with_file_name(name);
        let file = File::create_new(&temp)?;
        let result = self.write_to(file).and_then(|()| fs::rename(&temp, out));
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.map(|()| self.selected.coverage)
    }

    fn write_to(&mut self, file: File) -> io::Result<()> {
        let mut out = BufWriter::with_capacity(PIPE_BUFFER, file);
        self.selected.init.write(&mut out)?;
        let mut offset = self.selected.init.len() as u64;
        let first_dts = self.selected.fragments[0].sample.dts;
        for (index, fragment) in self.selected.fragments.iter().enumerate() {
            if fragment.sample.keyframe {
                let timestamp = self
                    .selected
                    .clock
                    .at(fragment.sample.pts)
                    .ok_or(io::ErrorKind::InvalidData)?;
                self.selected
                    .coverage
                    .keyframes
                    .push(KeyframeIndex { timestamp, offset });
            }
            fragment
                .sample
                .write(first_dts, (index + 1) as u32, &mut out)?;
            offset += fragment.sample.len() as u64;
        }
        out.flush()
    }
}

struct State {
    init: Option<Arc<Init>>,
    groups: VecDeque<Vec<Arc<BufferedFragment>>>,
    count: usize,
    clock: Option<SourceClock>,
    last_dts: Option<i64>,
    epoch: u64,
    gap: RecordingError,
    invalidated_by: RecordingError,
    finished: Option<RecordingError>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            init: None,
            groups: VecDeque::new(),
            count: 0,
            clock: None,
            last_dts: None,
            epoch: 0,
            gap: RecordingError::NoKeyframe,
            invalidated_by: RecordingError::NoKeyframe,
            finished: None,
        }
    }
}

impl State {
    fn reset(&mut self, reason: RecordingError) {
        self.groups.clear();
        self.count = 0;
        self.clock = None;
        self.last_dts = None;
        self.epoch += 1;
        self.gap = reason;
        self.invalidated_by = reason;
    }

    fn evict_oldest_gop(&mut self) -> bool {
        let Some(group) = self.groups.pop_front() else {
            return false;
        };
        self.count -= group.len();
        self.gap = RecordingError::BufferLimit;
        true
    }

    fn push(&mut self, sample: Fragment, budget: &Arc<AtomicUsize>, received: Arrival) {
        if sample.duration <= 0 || sample.duration > MAX_GAP_TICKS {
            self.reset(RecordingError::TimestampDiscontinuity);
            return;
        }
        if self
            .last_dts
            .is_some_and(|previous| !(2..=MAX_GAP_TICKS).contains(&(sample.dts - previous)))
        {
            self.reset(RecordingError::TimestampDiscontinuity);
        }
        if let Some(clock) = self.clock {
            self.clock = clock.observe(sample.dts, received);
            if self.clock.is_none() {
                self.reset(RecordingError::TimestampDiscontinuity);
            }
        }
        self.last_dts = Some(sample.dts);
        let cutoff = sample.dts - (BUFFER_TIME.as_secs() as i64 * TIMESCALE);
        while self
            .groups
            .front()
            .is_some_and(|group| group[0].sample.dts < cutoff)
        {
            self.evict_oldest_gop();
        }
        let charge = sample.capacity() + std::mem::size_of::<BufferedFragment>();
        // The byte counter includes old fragments pinned by clips being written.
        // Eviction always removes whole GOPs. A missing dependent frame means
        // dropping the rest of that GOP until the next keyframe.
        loop {
            if self.groups.is_empty() && !sample.keyframe {
                return;
            }
            if self.count < MAX_FRAGMENTS && reserve(budget, charge, BUFFER_BYTES).is_some() {
                break;
            }
            if !self.evict_oldest_gop() {
                self.gap = RecordingError::BufferLimit;
                return;
            }
        }

        self.clock.get_or_insert(SourceClock {
            anchor: received.wall,
            first_dts: sample.dts,
            first_pts: sample.pts,
            previous: received,
        });
        if sample.keyframe {
            self.groups.push_back(Vec::new());
        }
        self.groups
            .back_mut()
            .expect("keyframe group")
            .push(Arc::new(BufferedFragment {
                sample,
                charge,
                budget: Arc::clone(budget),
            }));
        self.count += 1;
    }

    fn reached(&self, at: SystemTime) -> bool {
        let Some(clock) = self.clock else {
            return false;
        };
        self.groups
            .back()
            .and_then(|group| group.last())
            .and_then(|f| clock.at(f.sample.dts))
            .is_some_and(|decoded| decoded >= at)
    }

    fn presentation_time(&self, fragment: &BufferedFragment) -> Option<SystemTime> {
        self.clock?.at(fragment.sample.pts)
    }

    fn select(
        &self,
        window: Window,
        stopped: Option<RecordingError>,
    ) -> Result<Selection, RecordingError> {
        let init = self
            .init
            .clone()
            .ok_or_else(|| stopped.unwrap_or(self.gap))?;
        let clock = self.clock.ok_or(self.gap)?;
        let first_group = self
            .groups
            .iter()
            .rposition(|group| {
                self.presentation_time(&group[0])
                    .is_some_and(|starts| starts <= window.start)
            })
            .unwrap_or_default();
        let start = self
            .groups
            .get(first_group)
            .and_then(|group| self.presentation_time(&group[0]))
            .ok_or(self.gap)?;
        if start > window.event {
            return Err(self.gap);
        }
        let mut end = start;
        let mut fragments = Vec::new();
        for fragment in self.groups.iter().skip(first_group).flatten() {
            let media_end = fragment.sample.pts + fragment.sample.duration;
            end = end.max(
                clock
                    .at(media_end)
                    .ok_or(RecordingError::TimestampDiscontinuity)?,
            );
            fragments.push(Arc::clone(fragment));
            if clock
                .at(fragment.sample.dts)
                .is_some_and(|at| at >= window.end)
            {
                break;
            }
        }
        if end < window.event {
            return Err(stopped.unwrap_or(self.gap));
        }
        let truncation = if end < window.end {
            stopped.or(Some(self.gap))
        } else {
            (start > window.start).then_some(self.gap)
        };
        let coverage = ClipCoverage {
            start,
            end,
            codec: init.codec,
            truncation,
            keyframes: Vec::new(),
        };
        Ok(Selection {
            init,
            fragments,
            coverage,
            clock,
        })
    }
}

struct Selection {
    init: Arc<Init>,
    fragments: Vec<Arc<BufferedFragment>>,
    coverage: ClipCoverage,
    clock: SourceClock,
}

struct BufferedFragment {
    sample: Fragment,
    charge: usize,
    budget: Arc<AtomicUsize>,
}

impl Drop for BufferedFragment {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.charge, Ordering::Relaxed);
    }
}

fn reserve(count: &AtomicUsize, amount: usize, max: usize) -> Option<()> {
    count
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
            used.checked_add(amount).filter(|&next| next <= max)
        })
        .ok()
        .map(|_| ())
}

#[derive(Clone, Copy)]
struct Arrival {
    wall: SystemTime,
    monotonic: Instant,
}

#[derive(Clone, Copy)]
struct SourceClock {
    anchor: SystemTime,
    first_dts: i64,
    first_pts: i64,
    previous: Arrival,
}

impl SourceClock {
    fn at(self, ticks: i64) -> Option<SystemTime> {
        self.offset_time(ticks.checked_sub(self.first_pts)?)
    }

    fn offset_time(self, offset: i64) -> Option<SystemTime> {
        let seconds = offset.unsigned_abs() / TIMESCALE as u64;
        let fraction = offset.unsigned_abs() % TIMESCALE as u64;
        let duration = Duration::new(
            seconds,
            (fraction * 1_000_000_000 / TIMESCALE as u64) as u32,
        );
        if offset >= 0 {
            self.anchor.checked_add(duration)
        } else {
            self.anchor.checked_sub(duration)
        }
    }

    fn observe(mut self, dts: i64, now: Arrival) -> Option<Self> {
        let elapsed = now
            .monotonic
            .saturating_duration_since(self.previous.monotonic);
        let wall = self.previous.wall.checked_add(elapsed)?;
        if wall
            .duration_since(now.wall)
            .unwrap_or_else(|e| e.duration())
            > MAX_WALL_CLOCK_JUMP
        {
            return None;
        }
        let projected = self.offset_time(dts.checked_sub(self.first_dts)?)?;
        match now.wall.duration_since(projected) {
            Ok(lag) if lag > MAX_INPUT_LAG => return None,
            // FFmpeg may first deliver several seconds together after probing.
            // Align their media timeline to the newest observed packet arrival.
            Err(lead) if lead.duration() > MAX_BATCH_LEAD => {
                self.anchor = self.anchor.checked_sub(lead.duration())?;
            }
            _ => {}
        }
        self.previous = now;
        Some(self)
    }
}
