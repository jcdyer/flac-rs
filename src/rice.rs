use bitwriter::BitWriter;

/// Rice encode a numeric value, putting the output in a bit stream.
///
/// TODO: Ensure this matches FLAC's expectations for rice format.
/// I.e: Sign bit (1 = positive), followed by base, followed by unary
/// overflow.  Unary encoding with zeros filled.  I suspect we use
/// zero-filled unary, since it would conflict less often with the sync
/// code.

/// FLAC Does not use a sign bit,but interleaves negative and positive values.
/// From the code comments at libflac/bitwriter.c:558
///
/// fold signed to uint32_t; actual formula is: negative(v)? -2v-1 : 2v

pub fn rice(order: usize, value: i64, w: &mut BitWriter) {
    // Interleave signed and unsigned values
    let value = if value >= 0 {
        2 * value
    } else {
        (-2 * value) - 1
    } as u64;

    let base = value & ((1 << order) - 1);
    let overflow = value >> order;

    // TODO: Make sure this compiles efficiently or manually unroll the loop.    w.put(1, !(sign_bit ^ positive)); // Put the sign bit;

    // Write the overflow in unary
    w.put(overflow as usize + 1, true);
    w.put(order, base); // Write the lower order bits in binary.
}
pub struct RiceEncoder {
    order: usize,
}

impl RiceEncoder {
    pub fn new(order: usize) -> RiceEncoder {
        RiceEncoder { order }
    }

    pub fn rice(&self, value: i64, w: &mut BitWriter) {
        rice(self.order, value, w)
    }
}

#[cfg(test)]
mod test {
    use bitwriter::BitWriter;

    use super::RiceEncoder;

    #[test]
    fn expected_sample() {

        let input: &[i64] = &[
            -5, 3, 1, -3, 6, -7, -4, 3, -2, 5, -10, 2, 2, -1, 10, 6, -2, 2, -4, 0, 3, -3, -3, -6,
            -4, 0, -1, 6, 3, 5, 8, 1, 3, 0, -3, -12, 0, -5, -1, -11, 2, -6, -2, 6, -1, 5, 7, 4, 13,
            3, 5, -6, -4, -6, -3, 3, 5, -5, -1, -1, 1, 3, 6, 2, -5, -2, -9, -1, 0, -6, 6, 0, -1, 2,
            -3, -7, -3, -4, 7, 0, 5, 4, 0, 0, 0, -3, 5, -5, 5, 4, 2, -3, -4, -2, 4, -1, 7, 3, -2,
            3, 4, -1, -3, -3, 0, -8, 1, 0, -9, 5, -3, 2, 2, 4, 3, 5, 0, -2, -3, -1, -5, 2, -3, -3,
            2, 0, -8, 10, -4, 4, -7, -4, -2, -1, 3, 7, 6, 1, 3, 3, -1, -7, 5, 0, -2, 1, 8, 1, 5,
            -2, 5, -2, -6, -1, -9, -1, -1, 1, 3, -4, -5, 3, -6, 5, 0, 2, 1, 0, 0, 1, -2, 2, 1, -6,
            -6, -10, 3, -3, 2, 5, -6, 7, 11, 10, 13, 4, 0, -8, -10,
        ];
        /*
          param = 2
          Original | Interleaved | Upper | Lower | Upper unary | Lower binary | Combined
          -5       | 9           | 2     | 1     | 001         | 01           | 00101
           3       | 6           | 1     | 2     | 01          | 10           | 0110
           1       | 2           | 0     | 2     | 1           | 10           | 110
          -3       | 5           | 1     | 1     | 01          | 01           | 0101
          0b0010_1011_0110_0101 = 0x2b 0x65
        */

        let expected_encoding: &[u8] = &[
            0x2b, 0x65, 0x10, 0x57, 0x6e, 0x60, 0xe8, 0x94, 0x10, 0x4e, 0x8f, 0x19, 0x54, 0xef,
            0x28, 0x8c, 0x60, 0x99, 0xa2, 0x83, 0xc2, 0xd0, 0x54, 0x3f, 0x12, 0x98, 0x62, 0x01,
            0x98, 0xc7, 0x73, 0xab, 0x18, 0xb6, 0xe6, 0x11, 0x0b, 0xc2, 0xd8, 0x71, 0x25, 0x45,
            0x15, 0x5c, 0x68, 0x62, 0x49, 0x14, 0xc5, 0x31, 0x11, 0x5f, 0x92, 0x8c, 0xdd, 0x89,
            0x55, 0x60, 0xfa, 0x05, 0x32, 0xa2, 0x11, 0x8d, 0x3a, 0xd2, 0xa2, 0xaa, 0x41, 0xc1,
            0x1c, 0x82, 0xbf, 0xac, 0x30, 0x99, 0x9a, 0x8a, 0x69, 0xf0, 0x4c, 0x6e, 0x6e, 0x7a,
            0x16, 0xdc, 0xce, 0x56, 0x39, 0xa2, 0x69, 0x37, 0x4c, 0x73, 0x87, 0x65, 0x43, 0x1c,
            0x60, 0x60, 0x40, 0x31, 0x20, 0xe1, 0xc0,
        ];
        let mut bw = BitWriter::new();
        let encoder = RiceEncoder::new(2);
        for value in input {
            encoder.rice(*value, &mut bw);
        }
        let bytes = bw.finish();
        assert_eq!(&bytes, expected_encoding);
    }
}
