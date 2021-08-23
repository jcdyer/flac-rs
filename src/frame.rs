use bytes::BufMut;

use crate::headers::{BlockSize, MetadataBlockStreamInfo};


pub enum BlockId {
    FixedStrategy { frame_number: u64 },
    VariableStrategy { sample_number: u64 },
}

enum ChannelLayout<S> {
    Independent {
        channels: Vec<Subframe<S>>,
    },
    MidSide {
        mid: Subframe<S>,
        side: Subframe<S>,
    },
    LeftSide {
        left: Subframe<S>,
        side: Subframe<S>,
    },
    RightSide {
        right: Subframe<S>,
        side: Subframe<S>,
    },
}

pub struct Frame<S> {
    header: FrameHeader,
    subframes: ChannelLayout<S>,

}

impl<S: Sample> Frame<S> {
    fn new(block_size: BlockSize, stream_info: &MetadataBlockStreamInfo, first_sample: u64) -> Option<Frame<S>> {
        (stream_info.bits_per_sample.inner() != S::bitsize()).then(|| Frame {
            header: FrameHeader {
                block_id: BlockId::FixedStrategy { frame_number: first_sample / block_size.inner() as u64 },
                nominal_block_size: 4096,
                sample_rate: 44100,
            },
            subframes: ChannelLayout::Independent { channels: Vec::new() }, // Set this later.
        })
    }
    fn set_subframe(&mut self, subframes: ChannelLayout<S>) {
        self.subframes = subframes;
    }
}

pub enum Subframe<S> {
    Constant { value: S },
    Verbatim { value: Vec<S> }, // Vec of len() == blocksize
}


pub struct FrameHeader {
    block_id: BlockId,
    /// Size of the block in samples.  In a FixedStrategy block,
    /// the last block may contain fewer samples than this.
    nominal_block_size: u16,
    sample_rate: u32, // SampleRate

}

impl FrameHeader {
    fn put_into<S: Sample>(&self, channel_layout: &ChannelLayout<S>, mut buf: impl BufMut) {
        let (blocking_strategy_bit, block_id) = match self.block_id {
            BlockId::FixedStrategy{ frame_number } => (0, frame_number),
            BlockId::VariableStrategy { sample_number } => (1, sample_number),
        };
        // Sync code + mandatory 0 + fixed
        buf.put_u16(0b1111_1111_1111_1000 + blocking_strategy_bit);
        // TODO: Use actual block size, not nominal?
        let block_size_bits: u8 = match self.nominal_block_size {
            192 => 0b0001,
            576 => 0b0010,
            1152 => 0b0011,
            2304 => 0b0100,
            4608 => 0b0101,
            256 => 0b1000,
            512 => 0b1001,
            1024 => 0b1010,
            2048 => 0b1011,
            4096 => 0b1100,
            8192 => 0b1101,
            16384 => 0b1110,
            32768 => 0b1111,
            x if x <= 256 => 0b0110, // 8 bit, stored at end of header as x - 1
            _ => 0b0111, // 16 bit, stored at end of header as x - 1
            _ => panic!(),
        };
        // Sample rate:
        let sample_rate = 0b0000; // 0b0000 = Read from STREAM_INFO
        buf.put_u8(block_size_bits << 4 & sample_rate);


    }
}
// This enum is going to make things slow....
pub trait Sample {
    fn bitsize() -> u8;
}

impl Sample for u16 {
    fn bitsize() -> u8 {
        16
    }
}