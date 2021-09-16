use std::{convert::TryInto, num::NonZeroU64};

use bitwriter::BitWriter;
use flac_rs::{
    encoder::{Block, FixedResidual},
    frame::{BlockId, ChannelLayout, Frame, FrameHeader, Subblock, Subframe},
    headers::{
        BitsPerSample, BlockSize, ChannelCount, FrameSize, MetadataBlockStreamInfo, SampleRate,
        SamplesInStream,
    },
};

// 192
static BLOCK855: &[u8] = &*include_bytes!("../data/block-855.raw");

static FRAME855: &[u8] = &*include_bytes!("../data/frame-855.flacframe");

#[test]
fn encode_fixed_mid_side() {
    let stream_info = MetadataBlockStreamInfo {
        min_block_size: BlockSize::new(192).unwrap(),
        max_block_size: BlockSize::new(192).unwrap(),
        min_frame_size: FrameSize::new(0).unwrap(),
        max_frame_size: FrameSize::new(0).unwrap(),
        sample_rate: SampleRate::new(44100).unwrap(),
        channels: ChannelCount::Two,
        bits_per_sample: BitsPerSample::new(16).unwrap(),
        samples_in_stream: SamplesInStream::Unknown,
        md5_signature: Default::default(),
    };
    let (left, right): (Vec<i16>, Vec<i16>) = BLOCK855
        .chunks_exact(4)
        .map(|x| {
            (
                i16::from_le_bytes(x[0..2].try_into().unwrap()),
                i16::from_le_bytes(x[2..4].try_into().unwrap()),
            )
        })
        .unzip();
    let left = Subblock { data: left };
    let right = Subblock { data: right };
    let block = Block::from_input(vec![left, right]);
    let (mid_subblock, side_subblock) = if let Block::Stereo {
        left,
        right,
        mid,
        side,
    } = block
    {
        (mid, side)
    } else {
        panic!("not stereo")
    };

    assert_eq!(mid_subblock.len(), 192);
    assert_eq!(side_subblock.len(), 192);
    let mid = Subframe::new_fixed(&mid_subblock.data, 2);
    let side = Subframe::new_fixed_from_widened(&side_subblock.data, 1)
        .expect("trying to code side channel");
    println!("mid: {:?}", mid);
    println!("side: {:?}", side);
    let mut frame = Frame::new(stream_info.min_block_size, &stream_info, 855 * 192).unwrap();
    frame.set_subframes(ChannelLayout::MidSide { mid, side });
    let mut w = BitWriter::new();
    frame.put_into(&mut w);
    assert_eq!(w.finish().as_ref(), FRAME855);
}
