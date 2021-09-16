#![allow(
    clippy::len_without_is_empty, // Types that are non-empty by construction do not need is_empty method
)]

pub mod encoder;
pub mod headers;

pub mod frame;
pub mod rice;
mod writer;
pub use writer::{FrameWriter, HeaderWriter};

pub const SMALL: bool = true;
pub const BLOCK_SIZE: u16 = if SMALL { 192 } else { 4096 };
