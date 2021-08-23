use std::{
    io::{self, Write},
    num::NonZeroU64,
};

use bytes::BufMut;

/// FLAC specifies a minimum block size of 16 and a maximum block size
/// of 65535, meaning the bit patterns corresponding to the numbers 0-15
/// in the minimum blocksize and maximum blocksize fields are invalid.
#[derive(Clone, Copy, Debug, Hash, Ord, Eq, PartialOrd, PartialEq)]
pub struct BlockSize(u16);

impl BlockSize {
    pub fn new(val: u16) -> Option<BlockSize> {
        (val >= 16).then(||BlockSize(val))
    }

    pub fn inner(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Hash, Ord, Eq, PartialOrd, PartialEq)]
pub struct FrameSize(u32); // From 24 bit input

impl FrameSize {
    pub fn new(val: u32) -> Option<FrameSize> {
        (val > 0 || val & 0xff000000 == 0).then(|| FrameSize(val))
    }

    pub fn inner(self) -> u32 {
        self.0
    }
}

/// Sample rate in Hz. Though 20 bits are available, the maximum
/// sample rate is limited by the structure of frame headers to
/// 655350Hz. Also, a value of 0 is invalid.
#[derive(Clone, Copy, Debug, Hash, Ord, Eq, PartialOrd, PartialEq)]
pub struct SampleRate(u32);

impl SampleRate {
    pub fn new(val: u32) -> Option<SampleRate> {
        (val > 0 && val <=655350).then(|| SampleRate(val))
    }

    pub fn inner(self) -> u32 {
        self.0
    }
}

/// FLAC supports from 1 to 8 channels
#[derive(Clone, Copy, Debug, Hash, Ord, Eq, PartialOrd, PartialEq)]
#[repr(u8)]
pub enum Channels {
    One = 1,
    Two = 2,
    Three = 3,
    Four = 4,
    Five = 5,
    Six = 6,
    Seven = 7,
    Eight = 8,
}

/// FLAC supports from 4 to 32 bits per sample. Currently the
/// reference encoder and decoders only support up to 24 bits
/// per sample.
#[derive(Clone, Copy, Debug, Hash, Ord, Eq, PartialOrd, PartialEq)]
pub struct BitsPerSample(u8);

impl BitsPerSample {
    pub fn new(val: u8) -> Option<BitsPerSample> {
        (4..33).contains(&val).then(|| BitsPerSample(val))
    }

    pub fn inner(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Hash, Ord, Eq, PartialOrd, PartialEq)]
pub enum SamplesInStream {
    Unknown,
    /// Up to 2^36 - 1
    Count(NonZeroU64),
}

impl SamplesInStream {
    pub fn new(val: u64) -> Option<SamplesInStream> {
        if val < 1 << 36 {
            Some(NonZeroU64::new(val).map_or(SamplesInStream::Unknown, SamplesInStream::Count))
        } else {
            None
        }
    }
    pub fn inner(self) -> u64 {
        match self {
            SamplesInStream::Unknown => 0,
            SamplesInStream::Count(n) => {
                if n.get() >= 1 << 36 {
                    // enum cannot enforce invariant directly
                    panic!("value out of range [0, 2^36)");
                } else {
                    n.get()
                }
            }
        }
    }
}

pub struct MetadataBlockStreamInfo {
    pub min_block_size: BlockSize,
    pub max_block_size: BlockSize,

    pub min_frame_size: FrameSize,
    pub max_frame_size: FrameSize,

    pub sample_rate: SampleRate,
    /// 3 bits.  Stored as number of channels - 1
    pub channels: Channels,

    // 5 bits. Stored as bits-per-sample - 1
    pub bits_per_sample: BitsPerSample,
    pub samples_in_stream: SamplesInStream,

    /// Calculated late in the process.
    pub md5_signature: md5::Md5,
}

impl MetadataBlockStreamInfo {
    pub fn put_into(&self, last_header: bool, mut buf: impl BufMut) {
        put_metadata_header(
            BLOCKTYPE_STREAMINFO,
            last_header,
            self.len() as u32,
            &mut buf,
        );
        buf.put_u16(self.min_block_size.inner()); // 2
        buf.put_u16(self.max_block_size.inner()); // 2
        put_u24(self.min_frame_size.inner(), &mut buf); // 3
        put_u24(self.max_frame_size.inner(), &mut buf); // 3
        let channels = self.channels as u8;
        // TODO: Check this logic.
        let packed = self.sample_rate.inner() << 12
            | (channels as u32 - 1) << 9
            | (self.bits_per_sample.inner() as u32) << 4
            | (self.samples_in_stream.inner() >> 32) as u32 & 0xf; // bits 36..32

        buf.put_u32(packed);
        buf.put_u32(self.samples_in_stream.inner() as u32); // least significant 32 bits
        buf.put_u128(0); // To be filled with MD5 sum during stream finalization
    }

    pub fn len(&self) -> usize {
        34
    }
}

pub struct MetadataBlockSeekTable {
    pub seekpoints: Vec<Seekpoint>,
}

pub struct Seekpoint {
    /// Sample number of first sample in the target frame
    sample_number: u64,
    /// Offset (in bytes) from the first byte of the first frame header to thefirst
    /// byte of the target frame
    byte_offset: u64,
    /// Number of samples in the target frame
    sample_count: u16,
}

pub struct MetadataBlockPadding {
    // Can be no more 2^24 - 1
    count: u32,
}

impl MetadataBlockPadding {
    pub fn new(count: u32) -> MetadataBlockPadding {
        if count > (1 << 24) - 1 {
            panic!("Padding header cannot be more than 2^24 - 1");
        }
        MetadataBlockPadding { count }
    }

    pub fn put_into(&self, last_header: bool, mut buf: impl BufMut) {
        put_metadata_header(BLOCKTYPE_PADDING, last_header, self.count, &mut buf);
        const BATCH_SIZE: usize = 1024;
        let zeros = [0; BATCH_SIZE];
        let mut written = 0;
        while written < self.count as usize - BATCH_SIZE {
            buf.put(&zeros[..]);
            written += BATCH_SIZE;
        }
        buf.put(&zeros[..self.count as usize - written]);
    }

    pub fn len(&self) -> usize {
        self.count as usize
    }
}

pub enum MetadataBlock {
    SeekTable(MetadataBlockSeekTable),
    Padding(MetadataBlockPadding),
}

impl MetadataBlock {
    pub fn put_into(&self, last_header: bool, buf: impl BufMut) {
        match self {
            MetadataBlock::SeekTable(seek_table) => todo!(),
            MetadataBlock::Padding(padding) => padding.put_into(last_header, buf),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            MetadataBlock::SeekTable(seek_table) => todo!(),
            MetadataBlock::Padding(padding) => todo!(),
        }
    }
}

const BLOCKTYPE_STREAMINFO: u8 = 0;
const BLOCKTYPE_PADDING: u8 = 1;
const BLOCKTYPE_APPLICATION: u8 = 2;
const BLOCKTYPE_SEEKTABLE: u8 = 3;
const BLOCKTYPE_VORBIS_COMMENT: u8 = 4;
const BLOCKTYPE_CUESHEET: u8 = 5;
const BLOCKTYPE_PICTURE: u8 = 6;
const BLOCKTYPE_INVALID: u8 = 127;

fn put_metadata_header(block_type: u8, last_header: bool, len: u32, mut buf: impl BufMut) {
    assert_ne!(block_type, BLOCKTYPE_INVALID);

    buf.put_u8(block_type | if last_header { 0x80 } else { 0 });
    put_u24(len, &mut buf);
}

///
fn put_u24(value: u32, mut buf: impl BufMut) {
    assert_eq!(value & 0xff000000, 0, "value must fit in 24 bits");
    buf.put_u8((value & 0xff0000 >> 16) as u8);
    buf.put_u16((value & 0xffff) as u16);
}
