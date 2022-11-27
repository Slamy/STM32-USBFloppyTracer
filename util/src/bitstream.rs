use crate::Bit;

use core::iter::Iterator;

extern crate alloc;
use alloc::boxed::Box;

pub struct BitStreamCollector<T>
where
    T: FnMut(u8),
{
    sink: T,
    bit_i: u8,
    working_byte: u8,
}

impl<T> BitStreamCollector<T>
where
    T: FnMut(u8),
{
    pub fn new(sink: T) -> BitStreamCollector<T> {
        BitStreamCollector {
            sink,
            bit_i: 0,
            working_byte: 0,
        }
    }

    pub fn feed(&mut self, cell: Bit) {
        {
            self.working_byte <<= 1;
            if cell.0 {
                self.working_byte |= 1;
            }
            self.bit_i += 1;
            if self.bit_i == 8 {
                self.bit_i = 0;
                (self.sink)(self.working_byte);
            }
        }
    }
}

pub fn to_bit_stream<T>(mut inval: u8, mut sink: T)
where
    T: FnMut(Bit),
{
    for _ in 0..8 {
        sink(Bit((inval & 0x80) != 0));
        inval <<= 1;
    }
}

pub struct BitStreamGenerator {
    feeder: Box<dyn Iterator<Item = u8>>,
    current_byte: u8,
    counter: u8,
}

impl BitStreamGenerator {
    pub fn new(feeder: Box<dyn Iterator<Item = u8>>) -> BitStreamGenerator {
        BitStreamGenerator {
            feeder,
            current_byte: 0,
            counter: 0,
        }
    }
}

impl Iterator for BitStreamGenerator {
    type Item = Bit;

    fn next(&mut self) -> Option<Self::Item> {
        if self.counter == 0 {
            //println!("Derp!");
            self.current_byte = self.feeder.next()?;

            //println!("Current Byte: {}",self.current_byte);
            self.counter = 8;
        }

        let current_bit = Bit((self.current_byte & 0x80) != 0);

        self.counter -= 1;
        self.current_byte = self.current_byte << 1;

        Some(current_bit)
    }
}

mod tests {
    use super::to_bit_stream;
    use super::Bit;
    use super::BitStreamGenerator;
    use alloc::vec;

    #[test]
    fn bitstream_test() {
        let v1: Vec<u8> = vec![0x55, 0xaa];
        let stream = BitStreamGenerator::new(Box::new(v1.into_iter()));
        let result: Vec<Bit> = stream.collect();
        println!("{:?}", result);
        assert_eq!(
            result,
            vec![
                false, true, false, true, false, true, false, true, // 0x55
                true, false, true, false, true, false, true, false, // 0xaa
            ]
        );
    }

    #[test]
    fn to_bit_stream_test() {
        let mut vout: Vec<u8> = Vec::new();

        let vin: Vec<u8> = vec![0xaa, 0x44, 0x89, 0x2a];
        for i in vin.iter() {
            to_bit_stream(*i, |d| vout.push(if d.0 { 1 } else { 0 }));
        }

        println!("{:?}", vout);

        assert_eq!(
            vout,
            vec![
                1, 0, 1, 0, 1, 0, 1, 0, //aa
                0, 1, 0, 0, 0, 1, 0, 0, //44
                1, 0, 0, 0, 1, 0, 0, 1, //89
                0, 0, 1, 0, 1, 0, 1, 0, //2a
            ]
        );
    }
}
