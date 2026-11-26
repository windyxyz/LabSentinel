//! Live sources retain one decoded frame while the analysis thread is busy.
//! File replay reads every frame synchronously.

use std::io;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime};

use crate::{recording::Recorder, ChildSlot, Frame, Ingest, IngestConfig};

pub struct CapturedFrame {
    pub frame: Frame,
    pub arrived: Instant,
    pub timestamp: SystemTime,
    pub overwritten: u64,
}

impl CapturedFrame {
    fn new(frame: Frame) -> Self {
        Self {
            frame,
            arrived: Instant::now(),
            timestamp: SystemTime::now(),
            overwritten: 0,
        }
    }
}

pub struct FrameReader(Source);

enum Source {
    Direct(Ingest),
    Live(LatestFrames),
}

impl FrameReader {
    pub fn start(cfg: IngestConfig, child: ChildSlot, live: bool) -> io::Result<Self> {
        Self::start_session(cfg, child, live, false, false).map(|(reader, _)| reader)
    }

    /// Opt into the original-encoded event buffer for this session.
    pub fn start_with_recording(
        cfg: IngestConfig,
        child: ChildSlot,
        live: bool,
        retina: bool,
    ) -> io::Result<(Self, Option<Recorder>)> {
        Self::start_session(cfg, child, live, true, retina)
    }

    fn start_session(
        cfg: IngestConfig,
        child: ChildSlot,
        live: bool,
        recording: bool,
        retina: bool,
    ) -> io::Result<(Self, Option<Recorder>)> {
        let mut ingest = Ingest::new(cfg);
        let recorder = ingest.spawn_session(Arc::clone(&child), recording, retina)?;
        let source = if live {
            Source::Live(LatestFrames::start(ingest, child)?)
        } else {
            Source::Direct(ingest)
        };
        Ok((Self(source), recorder))
    }

    pub fn read(&mut self) -> io::Result<Option<CapturedFrame>> {
        match &mut self.0 {
            Source::Direct(ingest) => ingest.read_frame().map(|f| f.map(CapturedFrame::new)),
            Source::Live(ingest) => ingest.read(),
        }
    }

    /// Called once after the decoder closes stdout for a finite file.
    pub fn file_exit_ok(&self) -> bool {
        matches!(&self.0, Source::Direct(ingest) if ingest.file_exit_ok())
    }
}

#[derive(Default)]
struct Pending {
    frame: Option<CapturedFrame>,
    error: Option<io::Error>,
    finished: bool,
}

impl Pending {
    fn replace(&mut self, mut frame: CapturedFrame) {
        if let Some(old) = self.frame.take() {
            frame.overwritten = old.overwritten + 1;
        }
        self.frame = Some(frame);
    }
}

type Mailbox = (Mutex<Pending>, Condvar);

struct Producer(Arc<Mailbox>);

impl Producer {
    fn drain(&self, mut ingest: Ingest) {
        let (data, ready) = &*self.0;
        loop {
            let result = ingest.read_frame();
            match result {
                Ok(Some(frame)) => {
                    let frame = CapturedFrame::new(frame);
                    data.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .replace(frame);
                    ready.notify_one();
                }
                result => {
                    data.lock().unwrap_or_else(|e| e.into_inner()).error = result.err();
                    return;
                }
            }
        }
    }
}

impl Drop for Producer {
    fn drop(&mut self) {
        let (data, ready) = &*self.0;
        data.lock().unwrap_or_else(|e| e.into_inner()).finished = true;
        ready.notify_one();
    }
}

struct LatestFrames {
    mailbox: Arc<Mailbox>,
    child: ChildSlot,
    thread: Option<JoinHandle<()>>,
}

impl LatestFrames {
    fn start(ingest: Ingest, child: ChildSlot) -> io::Result<Self> {
        let mailbox: Arc<Mailbox> = Arc::default();
        let producer = Producer(Arc::clone(&mailbox));
        let thread = std::thread::Builder::new()
            .name("live-capture".into())
            .spawn(move || {
                producer.drain(ingest);
            })?;
        Ok(Self {
            mailbox,
            child,
            thread: Some(thread),
        })
    }

    fn read(&self) -> io::Result<Option<CapturedFrame>> {
        let (data, ready) = &*self.mailbox;
        let guard = data.lock().unwrap_or_else(|e| e.into_inner());
        let mut pending = ready
            .wait_while(guard, |pending| {
                pending.frame.is_none() && !pending.finished
            })
            .unwrap_or_else(|e| e.into_inner());
        if let Some(frame) = pending.frame.take() {
            return Ok(Some(frame));
        }
        pending.error.take().map_or(Ok(None), Err)
    }
}

impl Drop for LatestFrames {
    fn drop(&mut self) {
        Ingest::kill_slot(&self.child);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_consumer_gets_latest_frame_and_exact_overwrite_count() {
        let mut pending = Pending::default();
        let frame = |index| {
            CapturedFrame::new(Frame {
                width: 1,
                height: 1,
                data: vec![0; 3],
                index,
            })
        };
        for index in 1..=3 {
            pending.replace(frame(index));
        }
        let latest = pending.frame.take().unwrap();
        assert_eq!(latest.frame.index, 3);
        assert_eq!(latest.overwritten, 2);
        pending.replace(frame(4));
        assert_eq!(pending.frame.take().unwrap().overwritten, 0);
    }
}
