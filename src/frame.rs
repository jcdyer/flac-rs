use std::{convert::TryInto, ops::{Add, Deref, Shr, Sub}};

use bitwriter::BitWriter;
use crc::{Algorithm, Crc};

use crate::{
    encoder::FixedResidual,
    headers::{BitsPerSample, BlockSize, MetadataBlockStreamInfo},
    rice::{find_optimum_rice_param, get_rice_encoding_length, rice},
};

pub enum BlockId {
    FixedStrategy { frame_number: u64 },
    VariableStrategy { sample_number: u64 },
}

pub enum ChannelLayout<S: Sample> {
    Independent {
        channels: Vec<Subframe<S>>,
    },
    MidSide {
        mid: Subframe<S>,
        side: Subframe<S::Widened>,
    },
    LeftSide {
        left: Subframe<S>,
        side: Subframe<S::Widened>,
    },
    SideRight {
        side: Subframe<S::Widened>,
        right: Subframe<S>,
    },
}

pub struct Frame<S: Sample> {
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
        match &self.subframes {
            ChannelLayout::Independent { channels } => {
                for subframe in channels {
                    subframe.put_into(w);
                }
            }
            ChannelLayout::MidSide { mid, side } => {
                mid.put_into(w);
                side.put_into(w);
            }
            ChannelLayout::LeftSide { left, side } => {
                left.put_into(w);
                side.put_into(w);
            },
            ChannelLayout::SideRight { side, right } => {
                side.put_into(w);
                right.put_into(w);
            },
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
    actual_block_size: u16,
    sample_rate: u32, // SampleRate
    bits_per_sample: BitsPerSample,
}

impl FrameHeader {
    fn put_into<S: Sample>(&self, channel_layout: &ChannelLayout<S>, w: &mut BitWriter) {
        w.flush(); // Flush before getting start offset for CRC
        let crc8_start = w.as_slice().len();
        let blocking_strategy_bit = matches!(self.block_id, BlockId::VariableStrategy { .. });
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
    }, // Vec with len() == frame size
    Fixed {
        predictor: Vec<S>,
        rice_param: usize,
        residual: Vec<i64>,
    },
}

impl<S: Sample> Subframe<S> {
    pub fn new_fixed(value: &[S], order: usize) -> Subframe<S> {
        let predictor = value[..order].to_owned();
        let residual: Vec<i64> = match order {
            1 => FixedResidual::<S, 1>::new(value).collect(),
            2 => FixedResidual::<S, 2>::new(value).collect(),
            3 => FixedResidual::<S, 3>::new(value).collect(),
            4 => FixedResidual::<S, 4>::new(value).collect(),
            _ => panic!("predictor order {} not supported.  Must be 1-4", order),
        };
        let rice_param = find_optimum_rice_param(&residual);
        Subframe::Fixed {
            predictor,
            residual,
            rice_param,
        }
    }

    pub fn new_fixed_from_widened(value: &[S::Widened], order: usize) -> Option<Subframe<S>> {
        let predictor = value[..order].iter().map(|&w| S::try_from_widened(w)).to_owned().collect::<Option<Vec<_>>>()?;
        let residual: Vec<i64> = match order {
            1 => FixedResidual::<S::Widened, 1>::new(value).collect(),
            2 => FixedResidual::<S::Widened, 2>::new(value).collect(),
            3 => FixedResidual::<S::Widened, 3>::new(value).collect(),
            4 => FixedResidual::<S::Widened, 4>::new(value).collect(),
            _ => panic!("predictor order {} not supported.  Must be 1-4", order),
        };
        let rice_param = find_optimum_rice_param(&residual);
        Some(Subframe::Fixed {
            predictor,
            residual,
            rice_param,
        })

    }
}

impl Subframe<i16> {
    pub fn from_subblock_i16(value: &[i16]) -> Subframe<i16> {
        let val = value[0];
        if value.iter().all(|sample| *sample == val) {
            Subframe::Constant { value: val }
        } else {
            let o1 = Subframe::new_fixed(value, 1);
            let o2 = Subframe::new_fixed(value, 2);
            let o3 = Subframe::new_fixed(value, 3);
            let o4 = Subframe::new_fixed(value, 4);
            let verbatim = Subframe::Verbatim {
                value: value.to_owned(),
            };

            let mut subframe = verbatim;
            for choice in [o1, o2, o3, o4] {
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
            subframe
        }
    }
}

impl<S: Sample> Subframe<S> {
    // Side channel cannot be encoded verbatim, because it does not generally fit in the
    // bit size of the frame.
    #[warn(clippy::logic_bug)]
    pub fn encode_side_channel(subblock: &Subblock<S::Widened>) -> Option<Subframe<S::Widened>> {
        let value = &subblock.data;
        let val = value[0];
        if false && value.iter().all(|sample| *sample == val) {
            //T TODO: This should probably return 16 bit values?
            Some(Subframe::Constant { value: val })
        } else {
            let o1 = Subframe::new_fixed(value, 1);
            let o2 = Subframe::new_fixed(value, 2);
            let o3 = Subframe::new_fixed(value, 3);
            let o4 = Subframe::new_fixed(value, 4);

            let mut subframe = o1;
            for choice in [o2, o3, o4] {
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
    }
}

impl<S: Sample> Subframe<S> {
    pub fn len(&self) -> usize {
        1 + match self {
            Subframe::Constant { .. } => S::bitsize() as usize / 8,
            Subframe::Verbatim { value } => value.len() * (S::bitsize() as usize / 8),
            Subframe::Fixed {
                predictor,
                residual,
                rice_param,
            } => {
                get_rice_encoding_length(residual, *rice_param)
                    + predictor.len() * S::bitsize() as usize / 8
            }
        }
    }
    pub(crate) fn from_subblock(subblock: &Subblock<S>) -> Subframe<S> {
        let value = &subblock.data;
            let val = value[0];
            if value.iter().all(|sample| *sample == val) {
                Subframe::Constant { value: val }
            } else {
                let o1 = Subframe::new_fixed(value, 1);
                let o2 = Subframe::new_fixed(value, 2);
                let o3 = Subframe::new_fixed(value, 3);
                let o4 = Subframe::new_fixed(value, 4);
                let verbatim = Subframe::Verbatim {
                    value: value.to_owned(),
                };

                let mut subframe = verbatim;
                for choice in [o1, o2, o3, o4] {
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
                subframe
            }
    }

}

impl<S: Sample> Subframe<S> {
    pub fn put_into(&self, w: &mut BitWriter) {
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
            Subframe::Constant { value } => w.put(S::bitsize() as usize, value.to_i64() as u64),
            Subframe::Verbatim { value } => {
                for sample in value {
                    w.put(S::bitsize() as usize, sample.to_i64() as u64);
                }
            }
            Subframe::Fixed {
                predictor,
                residual,
                rice_param,
            } => {
                for sample in predictor {
                    w.put(S::bitsize() as usize, sample.to_i64() as u64);
                }
                self.put_residual(residual, *rice_param, w);
            }
        }
    }

    fn put_residual(&self, residual: &[i64], rice_param: usize, w: &mut BitWriter) {
        let partition_order = 0u8; // TODO: Allow partitioning;
        w.put(2, false); // Residual coding method: 4 bit rice parameter
        w.put(4, partition_order);
        w.put(4, rice_param as u64);
        for value in residual {
            rice(rice_param, *value, w);
        }
    }
}

#[derive(Clone, Default)]
pub struct StackVec(usize, [u8; 16]);

impl From<&[u8]> for StackVec {
    fn from(slice: &[u8]) -> StackVec {
        let copylen = slice.len().min(16);

        let mut array = [0; 16];
        array[..copylen].copy_from_slice(&slice[..copylen]);
        StackVec(copylen, array)
    }
}

impl Deref for StackVec {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.1[..self.0]
    }
}

pub trait Sample: Copy + PartialEq + Add<Output=Self> + Shr<i32, Output=Self> + Sub<Output=Self> {
    const BITSIZE: usize;
    type Widened: Sample;

    fn bitsize() -> u8 {
        Self::BITSIZE as u8
    }
    fn write(&self, slice: &mut [u8]) -> Option<()> {
        if slice.len() < Self::BITSIZE / 8 {
            None
        } else {
            slice.copy_from_slice(&self.to_bytes());
            Some(())
        }
    }
    fn to_bytes(self) -> StackVec;
    fn to_i64(self) -> i64;
    fn widen(self) -> Self::Widened;
    fn try_from_widened(widened: Self::Widened) -> Option<Self>;
}

impl Sample for i16 {
    const BITSIZE: usize = 16;
    type Widened = i32;
    fn to_bytes(self) -> StackVec {
        self.to_be_bytes()[..].into()
    }
    fn to_i64(self) -> i64 {
        self as i64
    }
    fn widen(self) -> Self::Widened {
        self.into()
    }
    fn try_from_widened(widened: Self::Widened) ->Option<Self> {
        widened.try_into().ok()
    }
}

impl Sample for i32 {
    const BITSIZE: usize = 32;
    type Widened = i64;
    fn to_bytes(self) -> StackVec {
        self.to_be_bytes()[..].into()
    }

    fn to_i64(self) -> i64 {
        self as i64
    }

    fn widen(self) -> Self::Widened {
        self.into()
    }

    fn try_from_widened(widened: Self::Widened) ->Option<Self> {
        widened.try_into().ok()
    }

}

/// This only exists for widening side channel other sample types.  Widening this type will not work.
impl Sample for i64 {
    const BITSIZE: usize = 64;
    type Widened = i64;
    fn to_bytes(self) -> StackVec {
        self.to_be_bytes()[..].into()
    }

    fn to_i64(self) -> i64 {
        self
    }

    fn widen(self) -> Self::Widened {
        self
    }

    fn try_from_widened(widened: Self::Widened) ->Option<Self> {
        Some(widened)
    }
}

pub struct Subblock<S> {
    pub data: Vec<S>,
}

impl<S>  Subblock<S> {
    pub fn len(&self) -> usize {
        self.data.len()
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
