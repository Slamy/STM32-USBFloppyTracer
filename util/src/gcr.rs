// http://www.baltissen.org/newhtm/1541c.htm

use crate::Bit;

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
