/// Generate a minimal flac file, with  value: ()  value: () one block of zeroed data.

use std::{convert::TryInto, num::NonZeroU64};

use flac_rs::{BLOCK_SIZE, HeaderWriter, frame::{ChannelLayout, Frame, Subframe}, headers::{
        BitsPerSample, BlockSize, ChannelCount, FrameSize, MetadataBlockStreamInfo, SampleRate,
        SamplesInStream,
    }};

use md5::{Digest, Md5};

fn main() {
    let mut md5_signature = Md5::new();
    md5_signature.update([0u8; BLOCK_SIZE as usize * 2]);
    let mut stream_info = MetadataBlockStreamInfo {
        min_block_size: BlockSize::new(4096).unwrap(),
        max_block_size: BlockSize::new(4096).unwrap(),
        min_frame_size: FrameSize::new(0).unwrap(),
        max_frame_size: FrameSize::new(0).unwrap(),
        sample_rate: SampleRate::new(44100).unwrap(),
        channels: ChannelCount::One,
        bits_per_sample: BitsPerSample::new(16).unwrap(),
        samples_in_stream: SamplesInStream::Count(4096.try_into().unwrap()),
        md5_signature,
    };

    stream_info.samples_in_stream = SamplesInStream::Count(
        NonZeroU64::new(4096)
        .unwrap(),
    );
    assert_eq!(stream_info.bits_per_sample.inner(), 16);
    let frame_iter = std::iter::once({
        let mut frame = Frame::<i16>::new(stream_info.min_block_size, &stream_info, 0).unwrap();
        let layout = ChannelLayout::Independent { channels: vec![ Subframe::Constant { value: 0 }]};
        frame.set_subframes(layout);
        frame
    });


    let writer: HeaderWriter<_, i16> =
        HeaderWriter::new(std::fs::File::create("/tmp/out.flac").unwrap(), stream_info);
    let mut writer = writer
        .write_headers(std::iter::empty())
        .expect("writing headers");

    for frame in frame_iter {
        writer.write_frame(frame).expect("cannot write frame");
    }
    writer.finish().unwrap();
}
// 0A        C4        41
// 0000 1010 1100 0100 0100  - 44100 - sample rate
// 000 - 0 - channels - 1
// 00
// carry the last bit
// 1, 0000 - bits-per-sample 16
// carry four bits
// 00 00 10 00
// 0000, 0000 0000 0000 0000 0001 0000 0000 0000 -- Number of samples: 4096
//

