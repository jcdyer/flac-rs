use bitwriter::BitWriter;
use crc::{Algorithm, Crc};

use crate::{BLOCK_SIZE, encoder::FixedResidual, headers::{BitsPerSample, BlockSize, MetadataBlockStreamInfo}, rice::{RiceEncoder, find_optimum_rice_param, rice}};

pub enum BlockId {
    FixedStrategy { frame_number: u64 },
    VariableStrategy { sample_number: u64 },
}

pub enum ChannelLayout<S> {
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
    SideRight {
        side: Subframe<S>,
        right: Subframe<S>,
    },
}

impl<'a, S> IntoIterator for &'a ChannelLayout<S> {
    type IntoIter = ChannelLayoutIter<'a, S>;
    type Item = &'a Subframe<S>;

    fn into_iter(self) -> Self::IntoIter {
        ChannelLayoutIter {
            layout: self,
            idx: 0,
        }
    }
}


pub struct ChannelLayoutIter<'a, S> {
    layout: &'a ChannelLayout<S>,
    idx: usize,
}

impl<'a, S> Iterator for ChannelLayoutIter<'a, S> {
    type Item = &'a Subframe<S>;
    fn next(&mut self) -> Option<Self::Item> {
        let idx = self.idx;
        self.idx += 1;
        match self.layout {
            ChannelLayout::Independent { channels } => channels.get(idx),
            ChannelLayout::MidSide { mid, side } => match idx {
                0 => Some(mid),
                1 => Some(side),
                _ => None,
            },
            ChannelLayout::LeftSide { left, side } => match idx {
                0 => Some(left),
                1 => Some(side),
                _ => None,
            },

            ChannelLayout::SideRight { side, right } => match idx {
                0 => Some(side),
                1 => Some(right),
                _ => None,
            },
        }
    }
}

pub struct Frame<S> {
    header: FrameHeader,
    subframes: ChannelLayout<S>,
}

static FRAME_CRC16: Crc<u16> = Crc::<u16>::new(&Algorithm {
    check: 0,
    init: 0,
    poly: 0b1000_0000_0000_0101,
    refin: false,
    refout: false,
    residue: 0,
    xorout: 0,
});

impl<S: Sample> Frame<S> {
    pub fn new(
        block_size: BlockSize,
        stream_info: &MetadataBlockStreamInfo,
        first_sample: u64,
    ) -> Option<Frame<S>> {
        (stream_info.bits_per_sample.inner() == i16::bitsize()).then(|| Frame {
            header: FrameHeader {
                block_id: BlockId::FixedStrategy {
                    frame_number: first_sample / stream_info.min_block_size.inner() as u64,
                },
                nominal_block_size: stream_info.min_block_size.inner(),
                actual_block_size: block_size.inner(),
                sample_rate: 44100,
                bits_per_sample: stream_info.bits_per_sample,
            },
            subframes: ChannelLayout::Independent {
                channels: Vec::new(),
            }, // Set this later.
        })
    }

    pub fn set_subframes(&mut self, subframes: ChannelLayout<S>) {
        self.subframes = subframes;
    }
}

impl Frame<i16> {
    pub fn put_into(&self, w: &mut BitWriter) {
        w.flush();
        let crc16_start = w.as_slice().len();
        self.header.put_into(&self.subframes, w);
        for subframe in &self.subframes {
            subframe.put_into(w);
        }
        w.align_and_flush(); // Flush and align?
        let digest = FRAME_CRC16.checksum(&w.as_slice()[crc16_start..]);
        w.put(16, digest); // CRC of whole frame.
    }
}


static FRAME_HEADER_CRC8: Crc<u8> = Crc::<u8>::new(&Algorithm {
    check: 0,
    init: 0,
    poly: 0b0000_0111,
    refin: false,
    refout: false,
    residue: 0,
    xorout: 0,
});

pub struct FrameHeader {
    block_id: BlockId,
    /// Size of the block in samples.  In a FixedStrategy block,
    /// the last block may contain fewer samples than this.
    nominal_block_size: u16,
    actual_block_size: u16,
    sample_rate: u32, // SampleRate
    bits_per_sample: BitsPerSample,
}

impl FrameHeader {
    fn put_into<S: Sample>(&self, channel_layout: &ChannelLayout<S>, w: &mut BitWriter) {
        w.flush(); // Flush before getting start offset for CRC
        let crc8_start = w.as_slice().len();
        let blocking_strategy_bit = matches!(self.block_id, BlockId::VariableStrategy{..});
        // Sync code + mandatory 0
        w.put(15, 0b111_1111_1111_1100_u16);
        w.put(1, blocking_strategy_bit);
        let block_size_bits = match self.actual_block_size {
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
        let sample_rate_bits = match self.sample_rate {
            882000 => 0b0001u8,
            176400 => 0b0010,
            44100 => 0b1001,
            _ => {
                eprintln!(
                    "warning: unexpected sample rate: {}.  Deferring to STREAM_INFO header",
                    self.sample_rate
                );
                0b0000
            }
        }; // Read sample rate from STREAMINFO
        w.put(4, sample_rate_bits);
        w.put(
            4,
            match channel_layout {
                ChannelLayout::Independent { channels } => {
                    if channels.is_empty() || channels.len() > 8 {
                        panic!("No channels or too many channels.  Unsupported by FLAC.  (Handle this case when crating a channel layout).");
                    }
                    channels.len() as u8 - 1
                }
                ChannelLayout::LeftSide { .. } => 8,
                ChannelLayout::SideRight { .. } => 9,
                ChannelLayout::MidSide { .. } => 10,
            },
        );
        w.put(3, match self.bits_per_sample.inner() {
            8 => 0b001u8,
            12 => 0b010,
            16 => 0b100,
            20 => 0b101,
            24 => 0b110,
            _ => {
                eprintln!("warning: bitrate ({}) cannot be encoded in frame header.  Deferring to STREAM_INFO header", self.bits_per_sample.inner());
                0b000
            }
        });

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
            w.put(8, self.actual_block_size - 1);
        } else if block_size_bits == 0b0111 {
            w.put(16, self.actual_block_size - 1);
        }

        if sample_rate_bits == 0b1100 {
            w.put(8, self.sample_rate / 1000);
        } else if sample_rate_bits == 0b1101 {
            w.put(16, self.sample_rate);
        } else if sample_rate_bits == 0b1110 {
            w.put(16, self.sample_rate / 10);
        }
        w.flush(); // Flush before calculating digest
                   // TODO calculate this CRC as we go.
        let digest = FRAME_HEADER_CRC8.checksum(&w.as_slice()[crc8_start..]);
        w.put(8, digest);
    }
}

#[derive(Debug)]
pub enum Subframe<S> {
    Constant {
        value: S,
    },
    Verbatim {
        value: Vec<S>,
    }, // Vec with len() == blocksize
    Fixed {
        predictor: Vec<S>,
        residual: Vec<i64>,
    },
}

impl<S: Sample> Subframe<S> {
}

impl Subframe<i16> {
    // TODO: This forces a double encoding of all blocks.
    pub fn len(&self) -> usize {
        let mut scratch = BitWriter::with_capacity(BLOCK_SIZE as usize * i16::bitsize() as usize);
        self.put_into(&mut scratch);
        scratch.finish().len()
    }

    pub fn encode_subblock(subblock: &Subblock) -> Option<Subframe<i16>> {
        if let Subblock::I16(value) = subblock {
            let val = value[0];
            if value.iter().all(|sample| *sample == val) {
                Some(Subframe::Constant { value: val })
            } else {
                let o1 = Subframe::Fixed {
                    predictor: value[..1].to_owned(),
                    residual: FixedResidual::<1>::new(value).collect(),
                };
                let o2 = Subframe::Fixed {
                    predictor: value[..2].to_owned(),
                    residual: FixedResidual::<2>::new(value).collect(),
                };
                let o3 = Subframe::Fixed {
                    predictor: value[..3].to_owned(),
                    residual: FixedResidual::<3>::new(value).collect(),
                };
                let o4 = Subframe::Fixed {
                    predictor: value[..4].to_owned(),
                    residual: FixedResidual::<4>::new(value).collect(),
                };
                let verbatim = Subframe::Verbatim { value: value.to_owned() };
                // Arbitrary!
                let mut subframe = o1;
                for choice in [o2, o3, o4, verbatim] {
                    if choice.len() < subframe.len() {
                        subframe = choice;
                    }
                }
                /*
                match &subframe {
                    Subframe::Constant { value } => eprintln!("constant {:?}", value),
                    Subframe::Verbatim { .. } => eprintln!("verbatim"),
                    Subframe::Fixed { predictor, .. } => eprintln!("fixed: {:?}", predictor),
                }
                */
                Some(subframe)
            }
        } else {
            // Only Subblock::I16 is implemented now.
            None
        }
    }

    fn put_into(&self, w: &mut BitWriter) {
        w.put(1, false); // Zero bit padding;
        w.put(
            6,
            match self {
                Subframe::Constant { .. } => 0b000000u8,
                Subframe::Verbatim { .. } => 0b000001,
                Subframe::Fixed {
                    predictor: samples, ..
                } => 0b001000 | samples.len() as u8,
            },
        );
        w.put(1, false); // Wasted bits in source.  Not sure what this is used for.  Assume none for now.

        match self {
            Subframe::Constant { value } => w.put(i16::bitsize() as usize, *value as u16),
            Subframe::Verbatim { value } => {
                for sample in value {
                    w.put(i16::bitsize() as usize, *sample as u16);
                }
            }
            Subframe::Fixed {
                predictor,
                residual,
            } => {

                for sample in predictor {
                    w.put(i16::bitsize() as usize, *sample as u16);
                }
                self.put_residual(residual, w);
            }
        }
    }

    fn put_residual(&self, residual: &[i64], w: &mut BitWriter) {

        let partition_order = 0u8; // TODO: Allow partitioning;
        let rice_param = find_optimum_rice_param(residual);
        w.put(2, false); // Residual coding method: 4 bit rice parameter
        w.put(4, partition_order);
        w.put(4, rice_param as u64);
        for value in residual {
            rice(rice_param, *value, w);
        }
    }
}

pub trait Sample: Copy {
    const BITSIZE: usize;
    fn bitsize() -> u8 {
        Self::BITSIZE as u8
    }
    fn write(&self, slice: &mut [u8]) -> Option<()>;
}

impl Sample for i16 {
    const BITSIZE: usize = 16;
    fn write(&self, slice: &mut [u8]) -> Option<()> {
        if slice.len() < Self::BITSIZE / 8 {
            None
        } else {
            slice.copy_from_slice(&self.to_be_bytes());
            Some(())
        }
    }
}

pub enum Subblock {
    I16(Vec<i16>),
    U8(Vec<u8>),
    U24(Vec<u32>), // oof.
    U32(Vec<u32>),
}

impl Subblock {
    pub fn len(&self) -> usize {
        match self {
            Subblock::I16(v) => v.len(),
            Subblock::U8(v) => v.len(),
            Subblock::U24(v) => v.len(),
            Subblock::U32(v) => v.len(),
        }
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
