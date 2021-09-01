pub mod encoder;
pub mod headers;

pub mod frame;
mod writer;

pub use writer::{FrameWriter, HeaderWriter};

pub const BLOCK_SIZE: u16 = 192;
