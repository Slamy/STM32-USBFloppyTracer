use crate::Bit;
use crate::PulseDuration;
use core::iter::Iterator;

extern crate alloc;
use alloc::boxed::Box;

pub struct FluxPulseGenerator2<T>
where
    T: FnMut(PulseDuration),
{
    sink: T,
    pub cell_duration: u32,
    pulse_accumulator: u32,
}

impl<T> FluxPulseGenerator2<T>
where
    T: FnMut(PulseDuration),
{
    pub fn new(sink: T, cell_duration: u32) -> FluxPulseGenerator2<T> {
        FluxPulseGenerator2 {
            sink,
            cell_duration,
            pulse_accumulator: 0,
        }
    }

    pub fn feed(&mut self, cell: Bit) {
        self.pulse_accumulator += self.cell_duration;

        if cell.0 {
            (self.sink)(PulseDuration(self.pulse_accumulator as u16));

            self.pulse_accumulator = 0;
        }
    }
}

pub struct FluxPulseGenerator {
    pub feeder: Box<dyn Iterator<Item = Bit>>,
    cell_duration: u32,
    wait_for_start: bool,
}

impl FluxPulseGenerator {
    pub fn new(feeder: Box<dyn Iterator<Item = Bit>>, cell_duration: u32) -> FluxPulseGenerator {
        FluxPulseGenerator {
            feeder,
            cell_duration,
            wait_for_start: true,
        }
    }
}

impl Iterator for FluxPulseGenerator {
    type Item = PulseDuration;

    fn next(&mut self) -> Option<Self::Item> {
        let mut pulse_accumulator: u32 = 0;
        loop {
            let bit = self.feeder.next()?;

            pulse_accumulator += self.cell_duration;

            if bit.0 {
                if self.wait_for_start {
                    self.wait_for_start = false;
                    pulse_accumulator = 0;
                } else {
                    assert!(pulse_accumulator < 0xff00); // ensure that it fits in 16 bit
                    return Some(PulseDuration(pulse_accumulator as u16));
                }
            }
        }
    }
}

pub struct FluxPulseToCells2<T>
where
    T: FnMut(Bit),
{
    sink: T,
    pub cell_duration: u16,
}

impl<T> FluxPulseToCells2<T>
where
    T: FnMut(Bit),
{
    pub fn new(sink: T, cell_duration: u16) -> FluxPulseToCells2<T> {
        FluxPulseToCells2 {
            sink,
            cell_duration,
        }
    }

    pub fn feed(&mut self, mut duration: PulseDuration) {
        while duration.0 > (self.cell_duration + self.cell_duration / 2) {
            duration.0 -= self.cell_duration;
            (self.sink)(Bit(false));
        }

        (self.sink)(Bit(true));
    }
}

pub struct FluxPulseToCells<T>
where
    T: Iterator<Item = PulseDuration>,
{
    feeder: T,
    pub cell_duration: u32,
    pulse_accumulator: i32,
}

impl<T> FluxPulseToCells<T>
where
    T: Iterator<Item = PulseDuration>,
{
    pub fn new(feeder: T, cell_duration: u32) -> FluxPulseToCells<T> {
        FluxPulseToCells {
            feeder,
            cell_duration,
            pulse_accumulator: 1,
        }
    }
}

impl<T> Iterator for FluxPulseToCells<T>
where
    T: Iterator<Item = PulseDuration>,
{
    type Item = Bit;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pulse_accumulator < (self.cell_duration + self.cell_duration / 2) as i32 {
            let next = self.feeder.next()?;

            self.pulse_accumulator = next.0 as i32;
            if self.pulse_accumulator > self.cell_duration as i32 * 8 {
                self.pulse_accumulator = 0;
            }
            Some(Bit(true))
        } else {
            self.pulse_accumulator -= self.cell_duration as i32;

            Some(Bit(false))
        }
    }
}

mod tests {
    use super::*;

    #[test]
    fn cell_to_pulses_test() {
        let v1: Vec<u8> = vec![1, 0, 0, 1, 0, 1, 1, 0, 0, 0, 0, 1];
        let bit_source = Box::new(v1.into_iter().map(|x| Bit(x == 1)));
        let pulse_generator = FluxPulseGenerator::new(Box::new(bit_source), 100);
        let result: Vec<PulseDuration> = pulse_generator.collect();
        println!("{:?}", result);
        assert_eq!(
            result,
            vec![
                PulseDuration(300),
                PulseDuration(200),
                PulseDuration(100),
                PulseDuration(500)
            ]
        );
    }

    #[test]
    fn cell_to_pulses2_test() {
        let v1: Vec<u8> = vec![1, 0, 0, 1, 0, 1, 1, 0, 0, 0, 0, 1];
        let mut result: Vec<PulseDuration> = Vec::new();

        let mut pulse_generator = FluxPulseGenerator2::new(|f| result.push(f), 100);
        v1.into_iter()
            .for_each(|x| pulse_generator.feed(Bit(x == 1)));

        println!("{:?}", result);
        assert_eq!(
            result,
            vec![
                PulseDuration(100),
                PulseDuration(300),
                PulseDuration(200),
                PulseDuration(100),
                PulseDuration(500)
            ]
        );
    }

    #[test]
    fn pulse_to_cell2_test() {
        let range: Vec<i32> = vec![-49, -20, 0, 20, 49];

        for offset in range {
            let v1 = vec![
                PulseDuration((300 + offset) as u16),
                PulseDuration((200 + offset) as u16),
                PulseDuration((100 + offset) as u16),
                PulseDuration((500 + offset) as u16),
            ];

            let mut result: Vec<u32> = Vec::new();

            //result.
            let mut pulseparser = FluxPulseToCells2 {
                sink: |val| result.push(if val.0 { 1 } else { 0 }),
                cell_duration: 100,
            };
            v1.into_iter().for_each(|f| pulseparser.feed(f));

            println!("{:?}", result);
            assert_eq!(result, vec![0, 0, 1, 0, 1, 1, 0, 0, 0, 0, 1]);
        }
    }

    #[test]
    fn pulse_to_cell_test() {
        let range: Vec<i32> = vec![-49, -20, 0, 20, 49];

        for offset in range {
            let v1 = vec![
                PulseDuration((300 + offset) as u16),
                PulseDuration((200 + offset) as u16),
                PulseDuration((100 + offset) as u16),
                PulseDuration((500 + offset) as u16),
            ]
            .into_iter();
            let pulseparser = FluxPulseToCells {
                feeder: Box::new(v1),
                cell_duration: 100,
                pulse_accumulator: 0,
            };

            let result: Vec<u32> = pulseparser.map(|x| if x.0 { 1 } else { 0 }).collect();
            println!("{:?}", result);
            assert_eq!(result, vec![1, 0, 0, 1, 0, 1, 1, 0, 0, 0, 0]);
        }
    }
}
