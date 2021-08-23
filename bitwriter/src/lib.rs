use bytes::{BufMut, BytesMut};

#[derive(Debug)]
pub struct BitWriter {
    buf: BytesMut,
    scratch: Scratch,
    scratchptr: usize,
}

type Scratch = u64;
const SCRATCH_SIZE: usize = std::mem::size_of::<Scratch>() * 8;

impl BitWriter {
    pub fn new() -> BitWriter {
        BitWriter {
            buf: BytesMut::new(),
            scratch: 0,
            scratchptr: 0,
        }
    }

    pub fn with_capacity(n: usize) -> BitWriter {
        BitWriter {
            buf: BytesMut::with_capacity(n),
            scratch: 0,
            scratchptr: 0,
        }
    }

    pub fn put<T: Into<u64>>(&mut self, ct: usize, value: T) {
        let value = value.into();
        debug_assert!(self.scratchptr < SCRATCH_SIZE);

        let mut bits_remaining = ct;
        while bits_remaining > 0 {
            let batchsize = bits_remaining.min(SCRATCH_SIZE - self.scratchptr);
            bits_remaining -= batchsize;
            let mask = (1 as Scratch).wrapping_shl(batchsize as u32) - 1;
            let batch = (value >> bits_remaining) as Scratch & mask;
            self.scratch |= batch << (SCRATCH_SIZE - batchsize - self.scratchptr);
            self.scratchptr += batchsize;
            if self.scratchptr == SCRATCH_SIZE {
                self.flush();
            }
        }
    }

    fn flush(&mut self) {
        let to_write = self.scratchptr / 8;
        let remainder = self.scratchptr % 8;
        let mut bytes = self.scratch.to_be_bytes();
        self.buf.put(&bytes[..to_write]);
        if remainder > 0 {
            bytes[0] = bytes[to_write];
        } else {
            bytes[0] = 0;
        }
        for byte in bytes[1..].iter_mut() {
            *byte = 0;
        }
        self.scratch = Scratch::from_be_bytes(bytes);
        self.scratchptr = remainder;
    }

    pub fn finish(mut self) -> bytes::Bytes {
        self.flush();
        if self.scratchptr > 0 {
            self.buf.put_u8(self.scratch.to_be_bytes()[0]);
        }
        self.buf.freeze()
    }
}

#[cfg(test)]
mod tests {
    use super::BitWriter;

    #[test]
    fn write_bytes() {
        let mut writer = BitWriter::new();

        writer.put(32, 0xffffffffu32);
        writer.put(32, 0x1u32);
        let bytes = writer.finish();

        assert_eq!(&bytes, &[0xff, 0xff, 0xff, 0xff, 0, 0, 0, 1][..]);
    }

    #[test]
    fn write_across_scratch_boundary() {
        let mut writer = BitWriter::new();

        writer.put(16, 0xffffu16);
        writer.put(32, 0u8);
        writer.put(32, 0xffffffffu32);
        writer.put(16, 0u16);
        let bytes = writer.finish();

        assert_eq!(&bytes, &[0xff, 0xff, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0][..]);

    }
    #[test]
    fn write_partial_bytes() {
        let mut writer = BitWriter::new();

        writer.put(63, 0x7fff_ffff_ffff_ffffu64);
        writer.put(3, 7u8);
        writer.put(3, 0u8);
        writer.put(1, 1u8);
        let bytes = writer.finish();

        assert_eq!(&bytes, &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0b1100_0100][..]);
    }
}
