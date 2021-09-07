#![allow(
    clippy::from_iter_instead_of_collect, // I like calling from_iter, damnit.
)]

use std::{convert::TryInto, fs::File, iter::FromIterator, num::NonZeroU64, ops::Not};

use flac_rs::{
    encoder::Block,
    frame::Subblock,
    headers::{
        BitsPerSample, BlockSize, ChannelCount, FrameSize, MetadataBlockStreamInfo, SampleRate,
        SamplesInStream,
    },
    HeaderWriter, BLOCK_SIZE,
};

use md5::Md5;

fn main() {
    let wavfile = dbg!(std::env::args()).nth(1).unwrap();

    let mut wavfile = std::fs::File::open(wavfile).unwrap();
    let (wavheader, body) = wav::read(&mut wavfile).unwrap();
    let mut stream_info = streaminfo_from_wav(&wavheader).unwrap();

    stream_info.samples_in_stream = SamplesInStream::Count(
        NonZeroU64::new(
            match &body {
                wav::BitDepth::Eight(samples) => samples.len() as u64,
                wav::BitDepth::Sixteen(samples) => samples.len() as u64,
                wav::BitDepth::TwentyFour(samples) => samples.len() as u64,
                wav::BitDepth::ThirtyTwoFloat(samples) => samples.len() as u64,
                wav::BitDepth::Empty => panic!("empty wav file"),
            } / wavheader.channel_count as u64,
        )
        .unwrap(),
    );
    assert_eq!(stream_info.bits_per_sample.inner(), 16);
    let block_iter = body
        .as_sixteen()
        .expect("sixteen bit body")
        .chunks(flac_rs::BLOCK_SIZE as usize * stream_info.channels as usize)
        .map(|block| {
            let mut channels = vec![Vec::new(); stream_info.channels as usize];
            let mut i = 0;
            // Collate samples from input subblock, round robin style.
            for sample in block {
                channels[i].push(*sample);
                i = (i + 1) % stream_info.channels as u8 as usize;
            }
            Vec::from_iter(channels.into_iter().map(Subblock::I16))
        });
    let writer: HeaderWriter<_, i16> = HeaderWriter::new(
        std::fs::File::create("/tmp/out.flac").unwrap(),
        stream_info.clone(),
    );
    let mut writer = writer
        .write_headers(std::iter::empty())
        .expect("writing headers");
    for (blocknum, block) in block_iter.enumerate() {
        debug_assert!(block.is_empty().not());
        let block = Block::from_input(block);
        let frame = block
            .encode(&stream_info, blocknum as u64 * BLOCK_SIZE as u64)
            .expect("cannot create frame");
        writer.write_frame(frame).expect("cannot write frame");
    }
}

fn streaminfo_from_wav(wavheader: &wav::Header) -> Option<MetadataBlockStreamInfo> {
    Some(MetadataBlockStreamInfo {
        min_block_size: BlockSize::new(BLOCK_SIZE as u16)?,
        max_block_size: BlockSize::new(BLOCK_SIZE as u16)?,
        min_frame_size: FrameSize::new(0)?,
        max_frame_size: FrameSize::new(0)?,
        sample_rate: SampleRate::new(wavheader.sampling_rate)?,
        channels: ChannelCount::new(wavheader.channel_count)?,
        bits_per_sample: BitsPerSample::new(wavheader.bits_per_sample.try_into().ok()?)?,
        samples_in_stream: SamplesInStream::Unknown, // Set with info from body.
        md5_signature: Md5::default(),
    })
}
