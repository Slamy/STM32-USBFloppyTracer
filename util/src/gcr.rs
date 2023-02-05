// http://www.baltissen.org/newhtm/1541c.htm

use crate::Bit;

#[derive(Debug, PartialEq, Eq)]
pub enum GcrDecoderResult {
    Sync,
    Byte(u8),
}

const GCR_ENCODE_TABLE: [u8; 16] = [
    0b01010, //0000
    0b01011, //0001
    0b10010, //0010
    0b10011, //0011
    0b01110, //0100
    0b01111, //0101
    0b10110, //0110
    0b10111, //0111
    0b01001, //1000
    0b11001, //1001
    0b11010, //1010
    0b11011, //1011
    0b01101, //1100
    0b11101, //1101
    0b11110, //1110
    0b10101, //1111
];

// generated from GCR_ENCODE_TABLE through inversion
const GCR_DECODE_TABLE: [u8; 32] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 0, 1, 0, 12, 4, 5, 0, 0, 2, 3, 0, 15, 6, 7, 0, 9, 10, 11, 0, 13,
    14, 0,
];

pub fn to_gcr_stream<T>(byte: u8, mut sink: T)
where
    T: FnMut(Bit),
{
    let upper_nibble = byte >> 4;
    let lower_nibble = byte & 0xf;

    let mut gcr_word = (GCR_ENCODE_TABLE[upper_nibble as usize] as u16) << 5
        | GCR_ENCODE_TABLE[lower_nibble as usize] as u16;

    for _ in 0..10 {
        sink(Bit((gcr_word & (1 << 9)) != 0));
        gcr_word <<= 1;
    }
}

pub struct GcrDecoder<T>
where
    T: FnMut(GcrDecoderResult),
{
    sink: T,
    gcr_word_buffer: u64,
    shift_count: u8,
    in_sync: bool,
    ones_count: u32,
}

impl<T> GcrDecoder<T>
where
    T: FnMut(GcrDecoderResult),
{
    pub fn new(sink: T) -> Self {
        Self {
            sink,
            gcr_word_buffer: 0,
            shift_count: 0,
            in_sync: false,
            ones_count: 0,
        }
    }

    pub fn feed(&mut self, cell: Bit) {
        if cell.0 {
            self.ones_count += 1;
        } else {
            if self.ones_count >= 10 {
                self.in_sync = true;
                (self.sink)(GcrDecoderResult::Sync);
                self.shift_count = 0;
            }
            self.ones_count = 0;
        }

        if self.in_sync {
            self.gcr_word_buffer <<= 1;

            if cell.0 {
                self.gcr_word_buffer |= 1;
            }

            self.shift_count += 1;

            if self.shift_count >= 10 {
                self.shift_count = 0;
                let upper_word = (self.gcr_word_buffer >> 5) & 0b11111;
                let lower_word = self.gcr_word_buffer & 0b11111;

                let result = GCR_DECODE_TABLE[upper_word as usize] << 4
                    | GCR_DECODE_TABLE[lower_word as usize];
                (self.sink)(GcrDecoderResult::Byte(result));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::bitstream::to_bit_stream;

    use super::*;

    #[test]
    fn gcr_decoder_table_generator() {
        // use the GCR_ENCODE_TABLE to crate the GCR_DECODER_TABLE
        let combinations: Vec<_> = GCR_ENCODE_TABLE.iter().enumerate().collect();
        let mut decoder_table = [0_u8; 32];

        for i in combinations {
            println!("{} {}", i.0, i.1);
            decoder_table[*i.1 as usize] = i.0 as u8;
        }

        println!("{:?}", decoder_table);
        assert_eq!(GCR_DECODE_TABLE, decoder_table);
    }

    #[test]
    fn gcr_decoder_table_invertibility() {
        // just to be sure. try every combination
        for i in 0..16 {
            assert_eq!(i as u8, GCR_DECODE_TABLE[GCR_ENCODE_TABLE[i] as usize]);
        }
    }

    #[test]
    fn gcr_decoder_test() {
        let mut cells = Vec::new();

        to_bit_stream(0xff, |f| cells.push(f));
        to_bit_stream(0xff, |f| cells.push(f));
        to_bit_stream(0xff, |f| cells.push(f));

        // next GCR word must have a 0 at the beginning. A GCR encoded 0 fulfills this.
        to_gcr_stream(0x08, |f| cells.push(f));
        to_gcr_stream(0x42, |f| cells.push(f));

        // exactly 10 ones to be sure that it functions correctly
        to_bit_stream(0x03, |f| cells.push(f));
        to_bit_stream(0xff, |f| cells.push(f));

        to_gcr_stream(0x07, |f| cells.push(f));
        to_gcr_stream(0x12, |f| cells.push(f));
        to_gcr_stream(0x65, |f| cells.push(f));

        let mut result = Vec::new();
        let mut decoder = GcrDecoder::new(|f| result.push(f));

        cells.iter().for_each(|f| decoder.feed(*f));
        println!("{:02x?}", result);
        assert_eq!(
            result,
            vec![
                GcrDecoderResult::Sync,
                GcrDecoderResult::Byte(0x08),
                GcrDecoderResult::Byte(0x42),
                GcrDecoderResult::Byte(0x05), // this is an allowed artifact.
                GcrDecoderResult::Sync,
                GcrDecoderResult::Byte(0x07),
                GcrDecoderResult::Byte(0x12),
                GcrDecoderResult::Byte(0x65)
            ]
        );
    }
}
