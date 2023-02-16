use crate::Bit;

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
    pub fn new(sink: T) -> Self {
        Self {
            sink,
            bit_i: 0,
            working_byte: 0,
        }
    }

    pub fn feed(&mut self, cell: Bit) {
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

pub fn to_bit_stream<T>(mut inval: u8, mut sink: T)
where
    T: FnMut(Bit),
{
    for _ in 0..8 {
        sink(Bit((inval & 0x80) != 0));
        inval <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::to_bit_stream;
    use alloc::vec;

    #[test]
    fn to_bit_stream_test() {
        let mut vout: Vec<u8> = Vec::new();

        let vin: Vec<u8> = vec![0xaa, 0x44, 0x89, 0x2a];
        for i in &vin {
            to_bit_stream(*i, |d| vout.push(u8::from(d.0)));
        }

        println!("{vout:?}");

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
