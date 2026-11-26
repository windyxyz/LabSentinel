//! The single-video-track fMP4 format emitted by our FFmpeg pipe.

use std::io::{self, Read, Write};
use std::ops::Range;
use std::sync::Arc;

use super::{Recorder, RecordingError, VideoCodec};

pub(super) const TIMESCALE: i64 = 90_000;
const FILE_TYPE_LIMIT: usize = 4096;
const INIT_LIMIT: usize = 256 * 1024;
const MOOF_LIMIT: usize = 4096;
const PACKET_LIMIT: usize = 4 * 1024 * 1024;

pub(super) struct Init {
    pub(super) codec: VideoCodec,
    ftyp: Vec<u8>,
    moov: Vec<u8>,
}

impl Init {
    pub(super) fn len(&self) -> usize {
        self.ftyp.len() + self.moov.len()
    }

    pub(super) fn write(&self, mut out: impl Write) -> io::Result<()> {
        out.write_all(&self.ftyp)?;
        out.write_all(&self.moov)
    }
}

pub(super) struct Fragment {
    pub(super) dts: i64,
    pub(super) pts: i64,
    pub(super) duration: i64,
    pub(super) keyframe: bool,
    sequence: usize,
    decode_time: Range<usize>,
    moof: Vec<u8>,
    mdat: Vec<u8>,
}

impl Fragment {
    #[cfg(test)]
    pub(super) fn synthetic(dts: i64, duration: i64, keyframe: bool, bytes: usize) -> Self {
        Self {
            dts,
            pts: dts,
            duration,
            keyframe,
            sequence: 0,
            decode_time: 0..0,
            moof: Vec::new(),
            mdat: vec![0; bytes],
        }
    }

    pub(super) fn len(&self) -> usize {
        self.moof.len() + self.mdat.len()
    }

    pub(super) fn capacity(&self) -> usize {
        self.moof.capacity() + self.mdat.capacity()
    }

    pub(super) fn write(
        &self,
        first_dts: i64,
        sequence: u32,
        mut sink: impl Write,
    ) -> io::Result<()> {
        let mut moof = [0u8; MOOF_LIMIT];
        let moof = &mut moof[..self.moof.len()];
        moof.copy_from_slice(&self.moof);
        moof[self.sequence..self.sequence + 4].copy_from_slice(&sequence.to_be_bytes());
        let dts = (self.dts - first_dts).to_be_bytes();
        let start = dts.len() - self.decode_time.len();
        moof[self.decode_time.clone()].copy_from_slice(&dts[start..]);
        sink.write_all(moof)?;
        sink.write_all(&self.mdat)
    }
}

type Result<T> = std::result::Result<T, RecordingError>;
const INVALID: RecordingError = RecordingError::InvalidStream;

pub(super) fn read_stream(input: &mut impl Read, recorder: &Recorder) -> Result<()> {
    let first = Header::read(input)?.ok_or(RecordingError::Unavailable)?;
    let ftyp = first.read_box(input, b"ftyp", FILE_TYPE_LIMIT)?;
    let moov = Header::require(input)?.read_box(input, b"moov", INIT_LIMIT)?;
    let codec = video_codec(&moov)?;
    recorder.set_init(Arc::new(Init { codec, ftyp, moov }));

    while let Some(header) = Header::read(input)? {
        let moof = header.read_box(input, b"moof", MOOF_LIMIT)?;
        let (mut fragment, packet_size, packet_offset) = sample(moof)?;
        let media = Header::require(input)?;
        if media.size - 8 != packet_size || packet_offset != fragment.moof.len() + 8 {
            return Err(INVALID);
        }
        if media.size > PACKET_LIMIT {
            media.skip_box(input, b"mdat")?;
            recorder.discontinuity(RecordingError::BufferLimit);
            continue;
        }
        fragment.mdat = media.read_box(input, b"mdat", PACKET_LIMIT)?;
        recorder.feed(fragment);
    }
    Ok(())
}

struct Header {
    bytes: [u8; 8],
    size: usize,
}

impl Header {
    fn read(input: &mut impl Read) -> Result<Option<Self>> {
        let mut bytes = [0u8; 8];
        loop {
            match input.read(&mut bytes[..1]) {
                Ok(0) => return Ok(None),
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(INVALID),
            }
        }
        input.read_exact(&mut bytes[1..]).map_err(|_| INVALID)?;
        let size = u32_at(&bytes, 0)? as usize;
        if size < bytes.len() {
            return Err(INVALID);
        }
        Ok(Some(Self { bytes, size }))
    }

    fn require(input: &mut impl Read) -> Result<Self> {
        Self::read(input)?.ok_or(INVALID)
    }

    fn read_box(self, input: &mut impl Read, kind: &[u8; 4], limit: usize) -> Result<Vec<u8>> {
        if &self.bytes[4..] != kind || self.size > limit {
            return Err(INVALID);
        }
        let mut data = vec![0; self.size];
        data[..8].copy_from_slice(&self.bytes);
        input.read_exact(&mut data[8..]).map_err(|_| INVALID)?;
        Ok(data)
    }

    fn skip_box(self, input: &mut impl Read, kind: &[u8; 4]) -> Result<()> {
        if &self.bytes[4..] != kind {
            return Err(INVALID);
        }
        let payload = (self.size - 8) as u64;
        if io::copy(&mut input.take(payload), &mut io::sink()).map_err(|_| INVALID)? != payload {
            return Err(INVALID);
        }
        Ok(())
    }
}

fn child(bytes: &[u8], range: Range<usize>, kind: &[u8; 4]) -> Result<Range<usize>> {
    let mut start = range.start;
    while range.end.saturating_sub(start) >= 8 {
        let size = u32_at(bytes, start)? as usize;
        if size < 8 || size > range.end - start {
            return Err(INVALID);
        }
        if &bytes[start + 4..start + 8] == kind {
            return Ok(start + 8..start + size);
        }
        start += size;
    }
    Err(INVALID)
}

fn video_codec(moov: &[u8]) -> Result<VideoCodec> {
    let trak = child(moov, 8..moov.len(), b"trak")?;
    let mdia = child(moov, trak, b"mdia")?;
    let mdhd = child(moov, mdia.clone(), b"mdhd")?;
    let clock = &moov[mdhd];
    let timescale_at = match clock.first() {
        Some(0) => 12,
        Some(1) => 20,
        _ => return Err(INVALID),
    };
    if i64::from(u32_at(clock, timescale_at)?) != TIMESCALE {
        return Err(INVALID);
    }
    let minf = child(moov, mdia, b"minf")?;
    let stbl = child(moov, minf, b"stbl")?;
    let stsd = child(moov, stbl, b"stsd")?;
    let entries = &moov[stsd];
    if u32_at(entries, 4)? != 1 || u32_at(entries, 8)? as usize != entries.len() - 8 {
        return Err(INVALID);
    }
    let (codec, config, min_config_size) = match entries.get(12..16) {
        Some(b"avc1" | b"avc3") => (VideoCodec::H264, b"avcC", 7),
        Some(b"hev1" | b"hvc1") => (VideoCodec::Hevc, b"hvcC", 23),
        _ => return Err(RecordingError::UnsupportedCodec),
    };
    // The visual sample entry has a 78-byte fixed header before its child boxes.
    let config = child(entries, 16 + 78..entries.len(), config)?;
    if config.len() <= min_config_size || entries[config.start] != 1 {
        return Err(INVALID);
    }
    Ok(codec)
}

fn sample(moof: Vec<u8>) -> Result<(Fragment, usize, usize)> {
    let mfhd = child(&moof, 8..moof.len(), b"mfhd")?;
    let traf = child(&moof, 8..moof.len(), b"traf")?;
    if mfhd.len() != 8 {
        return Err(INVALID);
    }
    let tfhd = child(&moof, traf.clone(), b"tfhd")?;
    let tfdt = child(&moof, traf.clone(), b"tfdt")?;
    let trun = child(&moof, traf, b"trun")?;
    let mut header = Fields::new(&moof[tfhd]);
    let flags = header.next()?;
    // With default-base-is-moof and no absolute base data offset, fragments can
    // be moved independently and their trun data offsets stay valid.
    if flags & 0xff020001 != 0x020000 {
        return Err(INVALID);
    }
    header.next()?; // track id
    header.optional(flags, 0x02)?; // sample description index
    let default_duration = header.optional(flags, 0x08)?;
    let default_size = header.optional(flags, 0x10)?;
    let default_flags = header.optional(flags, 0x20)?;

    if tfdt.len() < 4 {
        return Err(INVALID);
    }
    let decode_time = tfdt.start + 4..tfdt.end;
    let dts = match (moof[tfdt.start], decode_time.len()) {
        (0, 4) => i64::from(u32_at(&moof, decode_time.start)?),
        (1, 8) => {
            let bytes = moof[decode_time.clone()].try_into().map_err(|_| INVALID)?;
            let dts = i64::from_be_bytes(bytes);
            if dts < 0 {
                return Err(INVALID);
            }
            dts
        }
        _ => return Err(INVALID),
    };

    let mut run = Fields::new(&moof[trun]);
    let flags = run.next()?;
    if run.next()? != 1 {
        return Err(INVALID);
    }
    let offset = run.optional(flags, 0x01)?.ok_or(INVALID)? as usize;
    let first_flags = run.optional(flags, 0x04)?;
    let duration = run
        .optional(flags, 0x100)?
        .or(default_duration)
        .ok_or(INVALID)?;
    let size = run
        .optional(flags, 0x200)?
        .or(default_size)
        .ok_or(INVALID)?;
    if size == 0 {
        return Err(INVALID);
    }
    let sample_flags = run
        .optional(flags, 0x400)?
        .or(first_flags)
        .or(default_flags)
        .ok_or(INVALID)?;
    let composition = run.optional(flags, 0x800)?.unwrap_or_default();
    let composition = match flags >> 24 {
        0 => i64::from(composition),
        1 => i64::from(composition as i32),
        _ => return Err(INVALID),
    };
    let pts = dts.checked_add(composition).ok_or(INVALID)?;
    let duration = i64::from(duration);
    pts.checked_add(duration).ok_or(INVALID)?;
    let fragment = Fragment {
        dts,
        pts,
        duration,
        keyframe: sample_flags & 0x0001_0000 == 0 && sample_flags & 0x0300_0000 != 0x0100_0000,
        sequence: mfhd.start + 4,
        decode_time,
        moof,
        mdat: Vec::new(),
    };
    Ok((fragment, size as usize, offset))
}

fn u32_at(bytes: &[u8], start: usize) -> Result<u32> {
    let bytes = bytes.get(start..start + 4).ok_or(INVALID)?;
    Ok(u32::from_be_bytes(bytes.try_into().map_err(|_| INVALID)?))
}

struct Fields<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Fields<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn next(&mut self) -> Result<u32> {
        let value = u32_at(self.bytes, self.position)?;
        self.position += 4;
        Ok(value)
    }

    fn optional(&mut self, flags: u32, mask: u32) -> Result<Option<u32>> {
        (flags & mask != 0).then(|| self.next()).transpose()
    }
}
