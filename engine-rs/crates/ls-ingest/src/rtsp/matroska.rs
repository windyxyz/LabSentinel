use std::io::{self, Write};

use crate::recording::VideoCodec;

pub(super) fn write_header(
    out: &mut impl Write,
    codec: VideoCodec,
    width: u32,
    height: u32,
    extra: &[u8],
    frame_rate: Option<(u32, u32)>,
) -> io::Result<()> {
    let mut header = Vec::new();
    let mut ebml = Vec::new();
    uint(&mut ebml, 0x4286, 1); // EBMLVersion
    uint(&mut ebml, 0x42f7, 1); // EBMLReadVersion
    uint(&mut ebml, 0x42f2, 4); // EBMLMaxIDLength
    uint(&mut ebml, 0x42f3, 8); // EBMLMaxSizeLength
    element(&mut ebml, 0x4282, b"matroska");
    uint(&mut ebml, 0x4287, 4); // DocTypeVersion
    uint(&mut ebml, 0x4285, 2); // DocTypeReadVersion
    element(&mut header, 0x1a45_dfa3, &ebml);
    // Streaming Segment with unknown total size.
    key(&mut header, 0x1853_8067);
    header.push(0xff);

    let mut info = Vec::new();
    uint(&mut info, 0x002a_d7b1, 1_000_000); // TimestampScale in nanoseconds
    element(&mut info, 0x4d80, b"LabSentinel");
    element(&mut info, 0x5741, b"LabSentinel");
    element(&mut header, 0x1549_a966, &info);

    let mut track = Vec::new();
    uint(&mut track, 0xd7, 1); // TrackNumber
    uint(&mut track, 0x73c5, 1); // TrackUID
    uint(&mut track, 0x83, 1); // video
    uint(&mut track, 0x9c, 0); // FlagLacing
    let codec_id = match codec {
        VideoCodec::H264 => b"V_MPEG4/ISO/AVC".as_slice(),
        VideoCodec::Hevc => b"V_MPEGH/ISO/HEVC".as_slice(),
    };
    element(&mut track, 0x86, codec_id);
    element(&mut track, 0x63a2, extra);
    if let Some((numerator, denominator)) = frame_rate.filter(|&(n, d)| n != 0 && d != 0) {
        uint(
            &mut track,
            0x0023_e383,
            u64::from(numerator) * 1_000_000_000 / u64::from(denominator),
        );
    }
    let mut video = Vec::new();
    uint(&mut video, 0xb0, u64::from(width));
    uint(&mut video, 0xba, u64::from(height));
    element(&mut track, 0xe0, &video);
    let mut tracks = Vec::new();
    element(&mut tracks, 0xae, &track);
    element(&mut header, 0x1654_ae6b, &tracks);
    out.write_all(&header)
}

pub(super) fn write_frame(
    out: &mut impl Write,
    cluster_ms: u64,
    relative_ms: i16,
    keyframe: bool,
    data: &[u8],
) -> io::Result<()> {
    let mut block = Vec::with_capacity(32);
    uint(&mut block, 0xe7, cluster_ms);
    block.push(0xa3); // SimpleBlock
    size(&mut block, 4 + data.len());
    block.push(0x81); // Track 1
    block.extend_from_slice(&relative_ms.to_be_bytes());
    block.push(if keyframe { 0x80 } else { 0 });

    let mut cluster = Vec::with_capacity(10 + block.len());
    key(&mut cluster, 0x1f43_b675);
    size(&mut cluster, block.len() + data.len());
    cluster.extend_from_slice(&block);
    out.write_all(&cluster)?;
    out.write_all(data)
}

fn key(out: &mut Vec<u8>, id: u32) {
    let bytes = id.to_be_bytes();
    out.extend_from_slice(&bytes[(id.leading_zeros() / 8) as usize..]);
}

fn size(out: &mut Vec<u8>, length: usize) {
    // Every element here is bounded by the frame or parameter size limit.
    let n = (1..=8)
        .find(|n| (length as u64) < (1u64 << (7 * n)) - 1)
        .unwrap();
    let encoded = ((1u64 << (7 * n)) | length as u64).to_be_bytes();
    out.extend_from_slice(&encoded[8 - n..]);
}

fn element(out: &mut Vec<u8>, id: u32, bytes: &[u8]) {
    key(out, id);
    size(out, bytes.len());
    out.extend_from_slice(bytes);
}

fn uint(out: &mut Vec<u8>, id: u32, value: u64) {
    let bytes = value.to_be_bytes();
    element(
        out,
        id,
        &bytes[((value.leading_zeros() / 8) as usize).min(7)..],
    );
}
