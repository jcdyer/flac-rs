use bitwriter::BitWriter;
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
    pub fn new(
        block_size: BlockSize,
        stream_info: &MetadataBlockStreamInfo,
        first_sample: u64,
    ) -> Option<Frame<S>> {
        (stream_info.bits_per_sample.inner() != S::bitsize()).then(|| Frame {
            header: FrameHeader {
                block_id: BlockId::FixedStrategy {
                    frame_number: first_sample / block_size.inner() as u64,
                },
                nominal_block_size: 4096,
                sample_rate: 44100,
            },
            subframes: ChannelLayout::Independent {
                channels: Vec::new(),
            }, // Set this later.
        })
    }
    fn set_subframes(&mut self, subframes: ChannelLayout<S>) {
        self.subframes = subframes;
    }

    pub fn put_into(&self, w: &mut BitWriter) {
        self.header.put_into(&self.subframes, w);
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
    fn put_into<S: Sample>(&self, channel_layout: &ChannelLayout<S>, w: &mut BitWriter) {
        let blocking_strategy_bit = match self.block_id {
            BlockId::FixedStrategy { .. } => false,
            BlockId::VariableStrategy { .. } => true,
        };
        // Sync code + mandatory 0 + fixed
        w.put(15, 0b111_1111_1111_1101_u16);
        w.put(1, blocking_strategy_bit);
        // TODO: Use actual block size, not nominal?
        let block_size_bits = match self.nominal_block_size {
            192 => 0b0001u8,
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
            _ => 0b0111,             // 16 bit, stored at end of header as x - 1
        };
        w.put(4, block_size_bits);
        let sample_rate_bits = 0u8;
        w.put(4, sample_rate_bits); // Read sample rate from STREAMINFO
        w.put(
            4,
            match channel_layout {
                ChannelLayout::Independent { channels } => {
                    if channels.len() == 0 || channels.len() > 8 {
                        panic!("Too many channels.  Unsupported by FLAC.  (Handle this case when crating a channel layout).");
                    }
                    channels.len() as u8 - 1
                }
                ChannelLayout::LeftSide { .. } => 8,
                ChannelLayout::RightSide { .. } => 9,
                ChannelLayout::MidSide { .. } => 10,
            },
        );
        // Read sample size from STREAMINFO
        w.put(3, 0u8);

        // Mandatory zero bit.  Aligns header at 32 bits written.
        w.put(1, false);

        let encoded_id = match self.block_id {
            BlockId::FixedStrategy { frame_number } => ftf8_encode(frame_number),
            BlockId::VariableStrategy { sample_number } => ftf8_encode(sample_number),
        };
        for byte in encoded_id {
            w.put(8, byte);
        }

        if block_size_bits == 0b0110 {
            w.put(8, self.nominal_block_size - 1);
        } else if block_size_bits == 0b0111 {
            w.put(16, self.nominal_block_size - 1);
        }

        if sample_rate_bits == 0b1100 {
            w.put(8, self.sample_rate / 1000);
        } else if sample_rate_bits == 0b1101 {
            w.put(16, self.sample_rate);
        } else if sample_rate_bits == 0b1110 {
            w.put(16, self.sample_rate / 10);
        }
        // TODO calculate this CRC as we go.
        let crc8_INVALID = 0u8;
        w.put(8, crc8_INVALID);
        todo!("UNFINISHED");
    }
}
pub trait Sample {
    fn bitsize() -> u8;
}

impl Sample for u16 {
    fn bitsize() -> u8 {
        16
    }
}

// FLAC-specific modified UTF-8 encoding for arbitrary number of bits.
fn ftf8_encode(mut val: u64) -> Vec<u8> {
    let mut buffer = [0; 8];
    let mut current = 7;
    let mut bits_to_fill = 6;
    if val < 128 {
        buffer[current] = val as u8;
    } else {
        while val >= 1 << bits_to_fill {
            buffer[current] = 0b1000_0000 | (val & 0b11_1111) as u8;
            val >>= 6;
            current -= 1;
            if bits_to_fill == 0 {
                panic!("Received a value that cannot be encoded with ftf8");
            } else {
                bits_to_fill -= 1;
            }
        }
        let prefix = match bits_to_fill {
            5 => 0b1100_0000,
            4 => 0b1110_0000,
            3 => 0b1111_0000,
            2 => 0b1111_1000,
            1 => 0b1111_1100,
            0 => 0b1111_1110,
            _ => unreachable!(),
        };
        let mask = (1 << bits_to_fill) - 1;
        buffer[current] = prefix | (val & mask) as u8;
    }
    buffer[current..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::ftf8_encode;

    #[test]
    #[should_panic]
    fn test_ftf8_encode_out_of_bounds() {
        ftf8_encode(1 << 36);
    }

    #[test]
    fn test_ftf_encode_in_bounds() {
        assert_eq!(&ftf8_encode(0), &[0]);
        assert_eq!(&ftf8_encode(1), &[1]);
        assert_eq!(&ftf8_encode(127), &[127]);
        assert_eq!(&ftf8_encode(128), &[0xc2, 0x80]);
        assert_eq!(&ftf8_encode(0x7ff), &[0xdf, 0xbf]);
        assert_eq!(&ftf8_encode(0x800), &[0xe0, 0xa0, 0x80]);
        assert_eq!(
            &ftf8_encode((1 << 36) - 1),
            &[0xfe, 0xbf, 0xbf, 0xbf, 0xbf, 0xbf, 0xbf],
        );
    }
}
