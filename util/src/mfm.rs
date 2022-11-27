use crate::Bit;
use core::iter::Iterator;

extern crate alloc;
use alloc::boxed::Box;

pub struct MfmEncoder {
    feeder: Box<dyn Iterator<Item = Bit>>,
    last_bit: Bit,
    current_bit: Bit,
    on_data_bit: bool,
}

impl Iterator for MfmEncoder {
    type Item = Bit;

    fn next(&mut self) -> Option<Self::Item> {
        let result;

        if self.on_data_bit {
            // Generate the MFM data bit. Just the value itself
            result = Some(self.current_bit);
        } else {
            // Generate the MFM clock bit. This depends on previous data.

            // First get the next data bit from the source
            self.last_bit = self.current_bit;

            // Didn't get any, then give None back.
            self.current_bit = self.feeder.next()?;

            // this is shortened for:
            // if last == 0 and current == 0 then 1 else 0
            result = Some(Bit(!self.last_bit.0 && !self.current_bit.0));
        }

        self.on_data_bit = !self.on_data_bit;

        result
    }
}

impl MfmEncoder {
    pub fn new(feeder: Box<dyn Iterator<Item = Bit>>) -> MfmEncoder {
        MfmEncoder {
            last_bit: Bit(false),
            current_bit: Bit(false),
            feeder,
            on_data_bit: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum MfmResult {
    Got(u8),
    Pending,
    SyncWord,
    Searching,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MfmResult2 {
    Got(u8),
    SyncWord,
}

pub struct MfmEncoder2<T>
where
    T: FnMut(Bit),
{
    sink: T,
    last_bit: Bit,
}

impl<T> MfmEncoder2<T>
where
    T: FnMut(Bit),
{
    pub fn new(sink: T) -> MfmEncoder2<T> {
        MfmEncoder2 {
            last_bit: Bit(false),
            sink,
        }
    }

    pub fn feed_encoded8(&mut self, mut val: u8) {
        for _ in 0..8 {
            if (val & 0x80) != 0 {
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
            val <<= 1;
        }
    }

    // TODO copy pasta
    pub fn feed_odd16_32(&mut self, mut val: u32) {
        for _ in 0..16 {
            if (val & 0x80000000) != 0 {
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

    pub fn feed(&mut self, inval: MfmResult2) {
        match inval {
            MfmResult2::Got(x) => self.feed_encoded8(x),
            MfmResult2::SyncWord => {
                self.feed_raw16(0x4489);
                self.last_bit = Bit(true);
            }
        }
    }
}

pub struct MfmDecoder2<T>
where
    T: FnMut(MfmResult2),
{
    sink: T,
    sync_buffer: u64,
    byte_buffer: u8,
    shift_count: u8,
    in_sync: bool,
    zero_count: i32,
}

impl<T> MfmDecoder2<T>
where
    T: FnMut(MfmResult2),
{
    pub fn new(sink: T) -> MfmDecoder2<T> {
        MfmDecoder2 {
            sink,
            sync_buffer: 0,
            byte_buffer: 0,
            shift_count: 0,
            in_sync: false,
            zero_count: 0,
        }
    }

    pub fn feed(&mut self, cell: Bit) {
        if cell.0 {
            self.zero_count = 0;
        } else {
            self.zero_count += 1;

            if self.zero_count >= 4 {
                self.in_sync = false;
            }
        }

        self.sync_buffer = (self.sync_buffer << 1) | (if cell.0 { 1 } else { 0 });
        if (self.sync_buffer & 0xffffffffffff) == 0x448944894489 {
            self.in_sync = true;
            self.shift_count = 0;
            self.byte_buffer = 0;
            (self.sink)(MfmResult2::SyncWord);
            return;
        }

        if self.in_sync {
            if (self.shift_count & 1) == 1 {
                self.byte_buffer <<= 1;
                self.byte_buffer |= if cell.0 { 1 } else { 0 };
            }
            self.shift_count += 1;
            if self.shift_count == 16 {
                self.shift_count = 0;
                (self.sink)(MfmResult2::Got(self.byte_buffer));
            }
        }
    }
}

pub struct MfmDecoder<T>
where
    T: Iterator<Item = Bit>,
{
    feeder: T,
    sync_buffer: u64,
    byte_buffer: u8,
    shift_count: u8,
    in_sync: bool,
    zero_count: i32,
}

impl<T> MfmDecoder<T>
where
    T: Iterator<Item = Bit>,
{
    pub fn new(feeder: T) -> MfmDecoder<T> {
        MfmDecoder {
            feeder,
            sync_buffer: 0,
            byte_buffer: 0,
            shift_count: 0,
            in_sync: false,
            zero_count: 0,
        }
    }
}

/*


 Iso Sync Word 0x4489
 Data  1 0 1 0 0 0 0 1   0xA1
 Clk  0 0 0 0 1 1 1 0
 MFM  0100010010101001   0x44A9 as it would be if encoded correctly
 Sync 0100010010001001   0x4489 is damaged to be detected separate to normal data.
*/
impl<T> Iterator for MfmDecoder<T>
where
    T: Iterator<Item = Bit>,
{
    type Item = MfmResult;
    fn next(&mut self) -> Option<Self::Item> {
        let next = self.feeder.next()?;

        self.zero_count += 1;
        if next.0 {
            self.zero_count = 0;
        }

        if self.zero_count >= 4 {
            self.in_sync = false;
        }

        self.sync_buffer = (self.sync_buffer << 1) | (if next.0 { 1 } else { 0 });
        if (self.sync_buffer & 0xffffffffffff) == 0x448944894489 {
            self.in_sync = true;
            self.shift_count = 0;
            self.byte_buffer = 0;
            return Some(MfmResult::SyncWord);
        }

        if self.in_sync {
            if (self.shift_count & 1) == 1 {
                self.byte_buffer <<= 1;
                self.byte_buffer |= if next.0 { 1 } else { 0 };
            }
            self.shift_count += 1;
            if self.shift_count == 16 {
                self.shift_count = 0;
                Some(MfmResult::Got(self.byte_buffer))
            } else {
                Some(MfmResult::Pending)
            }
        } else {
            Some(MfmResult::Searching)
        }
    }
}

mod tests {
    use super::*;

    #[test]
    fn mfm_encoder_test() {
        {
            let v1: Vec<u8> = vec![1, 1, 1];
            let bit_source = Box::new(v1.into_iter().map(|x| Bit(x == 1)));
            let encoder = MfmEncoder::new(bit_source);
            let result: Vec<u8> = encoder.map(|x| if x.0 { 1 } else { 0 }).collect();
            println!("{:?}", result);
            assert_eq!(result, vec![0, 1, 0, 1, 0, 1]);
        }

        {
            let v1: Vec<u8> = vec![0, 0, 0, 1, 1, 0, 1, 1];
            let bit_source = Box::new(v1.into_iter().map(|x| Bit(x == 1)));
            let encoder = MfmEncoder::new(bit_source);
            let result: Vec<u8> = encoder.map(|x| if x.0 { 1 } else { 0 }).collect();

            println!("{:?}", result);
            assert_eq!(result, vec![1, 0, 1, 0, 1, 0, 0, 1, 0, 1, 0, 0, 0, 1, 0, 1]);
        }
    }

    #[test]
    fn mfm_decoder_test() {
        let v1: Vec<u8> = vec![
            1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, // MFM 00
            1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, // MFM 00
            0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489, broken MFM A1
            0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489, broken MFM A1
            0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489, broken MFM A1
            0, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, // MFM 00
            0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, // MFM FE
            1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, 1, // MFM 01
            0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489, broken MFM A1
            0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489, broken MFM A1
            0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489, broken MFM A1
            0, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, // MFM 00
            0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, // MFM FE
            1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, 1, // MFM 01
        ];
        let bit_source = Box::new(v1.into_iter().map(|x| Bit(x == 1)));
        let decoder = MfmDecoder::new(Box::new(bit_source));

        let result: Vec<MfmResult> = decoder
            .filter(|f| matches!(f, MfmResult::Got(_) | MfmResult::SyncWord))
            .collect();
        assert_eq!(
            result,
            vec![
                MfmResult::SyncWord,
                MfmResult::Got(0),
                MfmResult::Got(0xfe),
                MfmResult::Got(1),
                MfmResult::Got(0xA1),
                MfmResult::Got(0xA1),
                MfmResult::SyncWord,
                MfmResult::Got(0),
                MfmResult::Got(0xfe),
                MfmResult::Got(1),
            ]
        );
    }

    #[test]
    fn mfm_decoder2_test() {
        let v1: Vec<u8> = vec![
            1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, // MFM 00
            1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, // MFM 00
            0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489, broken MFM A1
            0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489, broken MFM A1
            0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489, broken MFM A1
            0, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, // MFM 00
            0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, // MFM FE
            1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, 1, // MFM 01
            0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489, broken MFM A1
            0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489, broken MFM A1
            0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, // Sync Word 4489, broken MFM A1
            0, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, // MFM 00
            0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, // MFM FE
            1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0, 1, // MFM 01
        ];
        let mut result: Vec<MfmResult2> = Vec::new();
        let mut decoder = MfmDecoder2::new(|val| result.push(val));

        v1.into_iter()
            .map(|x| Bit(x == 1))
            .for_each(|cell| decoder.feed(cell));

        println!("{:?}", result);

        assert_eq!(
            result,
            vec![
                MfmResult2::SyncWord,
                MfmResult2::Got(0),
                MfmResult2::Got(0xfe),
                MfmResult2::Got(1),
                MfmResult2::Got(0xA1),
                MfmResult2::Got(0xA1),
                MfmResult2::SyncWord,
                MfmResult2::Got(0),
                MfmResult2::Got(0xfe),
                MfmResult2::Got(1),
            ]
        );
    }

    #[test]
    fn mfm_encoder2_test() {
        let input = vec![
            MfmResult2::SyncWord,
            MfmResult2::SyncWord,
            MfmResult2::Got(0),
            MfmResult2::Got(0xfe),
            MfmResult2::Got(1),
            MfmResult2::SyncWord,
            MfmResult2::SyncWord,
            MfmResult2::SyncWord,
            MfmResult2::Got(0),
            MfmResult2::Got(0xfe),
            MfmResult2::Got(1),
        ];
        let mut result: Vec<u8> = Vec::new();
        let mut encoder = MfmEncoder2::new(|val| result.push(if val.0 { 1 } else { 0 }));

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
