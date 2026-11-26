//! Optional RTSP/RTP receive path. The decoder and evidence muxer consume the
//! same timestamped packets through a private, single-track Matroska pipe.

mod matroska;

use futures_util::StreamExt;
use percent_encoding::percent_decode_str;
use retina::client::{Credentials, Demuxed, PlayOptions, Session, SessionOptions, SetupOptions};
use retina::codec::{CodecItem, FrameFormat, ParametersRef, VideoFrame};
use std::io::{self, Write};
use std::process::ChildStdin;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::timeout;
use url::Url;

use crate::recording::VideoCodec;

const RTSP_DEADLINE: Duration = Duration::from_secs(7);
const MAX_ACCESS_UNIT: usize = 16 * 1024 * 1024;

pub(crate) struct Pump {
    stop: oneshot::Sender<()>,
    thread: JoinHandle<()>,
}

impl Pump {
    pub fn start(source: String, mut stdin: ChildStdin) -> io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (stop, cancelled) = oneshot::channel();
        let thread = thread::Builder::new()
            .name("rtsp-retina".into())
            .spawn(move || {
                runtime.block_on(async {
                    tokio::select! {
                        result = receive(&source, &mut stdin) => {
                            if let Err(reason) = result {
                                eprintln!("[ls-ingest] Retina: {reason}");
                            }
                        }
                        _ = cancelled => {}
                    }
                });
            })?;
        Ok(Self { stop, thread })
    }

    /// The FFmpeg child must already be closed so a blocked write can return.
    pub fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.thread.join();
    }
}

fn connection(source: &str) -> Result<(Url, Option<Credentials>), &'static str> {
    let mut url = Url::parse(source).map_err(|_| "invalid RTSP address")?;
    if url.scheme() != "rtsp" || url.host().is_none() {
        return Err("RTSP/TCP is required");
    }
    let decode = |part: &str| {
        percent_decode_str(part)
            .decode_utf8()
            .map(|value| value.into_owned())
            .map_err(|_| "invalid RTSP credentials")
    };
    let creds = if url.username().is_empty() && url.password().is_none() {
        None
    } else {
        let creds = Credentials {
            username: decode(url.username())?,
            password: decode(url.password().unwrap_or_default())?,
        };
        url.set_password(None).map_err(|_| "invalid RTSP address")?;
        url.set_username("").map_err(|_| "invalid RTSP address")?;
        Some(creds)
    };
    Ok((url, creds))
}

async fn receive(source: &str, output: &mut impl Write) -> Result<(), &'static str> {
    let (url, creds) = connection(source)?;
    let (mut stream, index, codec) = timeout(RTSP_DEADLINE, async {
        let mut session = Session::describe(
            url,
            SessionOptions::default()
                .creds(creds)
                .user_agent("LabSentinel Retina".to_owned()),
        )
        .await
        .map_err(|_| "RTSP DESCRIBE failed")?;
        let (index, codec) = session
            .streams()
            .iter()
            .enumerate()
            .find_map(
                |(index, stream)| match (stream.media(), stream.encoding_name()) {
                    ("video", "h264") => Some((index, VideoCodec::H264)),
                    ("video", "h265") => Some((index, VideoCodec::Hevc)),
                    _ => None,
                },
            )
            .ok_or("no supported H.264/H.265 video track")?;
        session
            .setup(
                index,
                SetupOptions::default().frame_format(FrameFormat::MP4),
            )
            .await
            .map_err(|_| "RTSP SETUP failed")?;
        let stream = session
            .play(PlayOptions::default())
            .await
            .map_err(|_| "RTSP PLAY failed")?
            .demuxed()
            .map_err(|_| "RTSP depacketizer unavailable")?;
        Ok::<_, &'static str>((stream, index, codec))
    })
    .await
    .map_err(|_| "RTSP handshake timed out")??;

    let first = timeout(RTSP_DEADLINE, async {
        loop {
            let frame = video_frame(&mut stream).await?;
            if frame.is_random_access_point() && frame.loss() == 0 {
                return Ok::<_, &'static str>(frame);
            }
        }
    })
    .await
    .map_err(|_| "no clean keyframe before deadline")??;
    let Some(ParametersRef::Video(params)) = stream.streams()[index].parameters() else {
        return Err("video parameters missing");
    };
    let (w, h) = params.pixel_dimensions();
    let extra = params.extra_data();
    if w == 0 || h == 0 || extra.is_empty() || extra.len() > 64 * 1024 {
        return Err("invalid video parameters");
    }
    matroska::write_header(output, codec, w, h, extra, params.frame_rate())
        .map_err(|_| "decoder pipe closed")?;

    let origin = first.timestamp().timestamp();
    write_frame(output, 0, 0, first)?;
    let mut latest = 0;
    loop {
        let frame = timeout(RTSP_DEADLINE, video_frame(&mut stream))
            .await
            .map_err(|_| "RTSP video timed out")??;
        if frame.loss() != 0 || frame.has_new_parameters() {
            return Err("RTSP video parameters changed or packets were lost");
        }
        let ts = frame.timestamp();
        let position = (i128::from(ts.timestamp()) - i128::from(origin)) * 1000
            / i128::from(ts.clock_rate().get());
        let pts = i64::try_from(position).map_err(|_| "RTSP timestamp out of range")?;
        if pts < latest - 3_000 || pts > latest + 10_000 {
            return Err("RTSP timestamp discontinuity");
        }
        // A leading picture after an open-GOP keyframe may refer to prior media.
        if pts >= 0 {
            latest = latest.max(pts);
            write_frame(output, latest as u64, (pts - latest) as i16, frame)?;
        }
    }
}

async fn video_frame(stream: &mut Demuxed) -> Result<VideoFrame, &'static str> {
    loop {
        match stream.next().await {
            Some(Ok(CodecItem::VideoFrame(frame))) if frame.data().len() <= MAX_ACCESS_UNIT => {
                return Ok(frame)
            }
            Some(Ok(CodecItem::VideoFrame(_))) => return Err("video access unit exceeds limit"),
            Some(Ok(_)) => {}
            Some(Err(_)) => return Err("RTSP packet read failed"),
            None => return Err("RTSP stream ended"),
        }
    }
}

fn write_frame(
    output: &mut impl Write,
    cluster: u64,
    offset: i16,
    frame: VideoFrame,
) -> Result<(), &'static str> {
    matroska::write_frame(
        output,
        cluster,
        offset,
        frame.is_random_access_point(),
        frame.data(),
    )
    .map_err(|_| "decoder pipe closed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_percent_encoded_credentials_without_leaving_them_in_url() {
        let (url, creds) = connection("rtsp://lab%40a:p%3A%23@127.0.0.1/stream?q=1").unwrap();
        let creds = creds.unwrap();
        assert_eq!(url.as_str(), "rtsp://127.0.0.1/stream?q=1");
        assert_eq!(creds.username, "lab@a");
        assert_eq!(creds.password, "p:#");
        assert!(connection("rtsps://localhost/stream").is_err());
    }
}
