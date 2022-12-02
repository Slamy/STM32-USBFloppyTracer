use crate::Bit;
extern crate alloc;

#[derive(Debug, PartialEq, Eq)]
pub enum MfmWord {
    Enc(u8),
    SyncWord,
}

pub struct MfmEncoder<T>
where
    T: FnMut(Bit),
{
    sink: T,
    last_bit: Bit,
}

const ISO_SYNC_WORD: u16 = 0x4489;

/*
 Iso Sync Word 0x4489
 Data  1 0 1 0 0 0 0 1   0xA1
 Clk  0 0 0 0 1 1 1 0
 MFM  0100010010101001   0x44A9 as it would be if encoded correctly
 Sync 0100010010001001   0x4489 is damaged to be detected separate to normal data.
*/
impl<T> MfmEncoder<T>
where
    T: FnMut(Bit),
{
    pub fn new(sink: T) -> MfmEncoder<T> {
        MfmEncoder {
            last_bit: Bit(false),
            sink,
        }
    }

    fn encode_mfm_bit(&mut self, val: u32, mask: u32) {
        if (val & mask) != 0 {
            // Encode 1
            (self.sink)(Bit(false)); // Clock Bit 0
            (self.sink)(Bit(true)); // Data Bit 1
            self.last_bit = Bit(true);
        } else {
            // Encode 0
            if self.last_bit.0 == true {
                (self.sink)(Bit(false));
                (self.sink)(Bit(false));
            } else {
                (self.sink)(Bit(true));
                (self.sink)(Bit(false));
            }
            self.last_bit = Bit(false);
        }
    }

    pub fn feed_encoded8(&mut self, mut val: u8) {
        for _ in 0..8 {
            self.encode_mfm_bit(val as u32, 1 << 7);
            val <<= 1;
        }
    }

    pub fn feed_odd16_32(&mut self, mut val: u32) {
        for _ in 0..16 {
            self.encode_mfm_bit(val, 1 << 31);
            val <<= 2;
        }
    }

    pub fn feed_even16_32(&mut self, val: u32) {
        self.feed_odd16_32(val << 1);
    }

    pub fn feed_raw16(&mut self, mut val: u16) {
        self.last_bit = Bit((val & 0x0001) != 0);

        for _ in 0..16 {
            (self.sink)(Bit((val & 0x8000) != 0));
            val <<= 1;
        }
    }

    pub fn feed(&mut self, inval: MfmWord) {
        match inval {
            MfmWord::Enc(x) => self.feed_encoded8(x),
            MfmWord::SyncWord => {
                self.feed_raw16(ISO_SYNC_WORD);
                self.last_bit = Bit(true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mfm_encoder2_test() {
        let input = vec![
            MfmWord::SyncWord,
            MfmWord::SyncWord,
            MfmWord::Enc(0),
            MfmWord::Enc(0xfe),
            MfmWord::Enc(1),
            MfmWord::SyncWord,
            MfmWord::SyncWord,
            MfmWord::SyncWord,
            MfmWord::Enc(0),
            MfmWord::Enc(0xfe),
            MfmWord::Enc(1),
        ];
        let mut result: Vec<u8> = Vec::new();
        let mut encoder = MfmEncoder::new(|val| result.push(if val.0 { 1 } else { 0 }));

        input.into_iter().for_each(|cell| encoder.feed(cell));

        println!("{:?}", result);
        assert_eq!(
            result,
            vec![
                0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489,
                0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489,
                0, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, // MFM 00
                0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, // MFM FE
                1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, 1, // MFM 01
                0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489,
                0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489,
                0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489,
                0, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, // MFM 00
                0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, // MFM FE
                1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, 1, // MFM 01
            ]
        );
    }
}
