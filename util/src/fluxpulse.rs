use crate::Bit;
use crate::PulseDuration;

extern crate alloc;

pub struct FluxPulseGenerator<T>
where
    T: FnMut(PulseDuration),
{
    sink: T,
    pub cell_duration: u32,
    pulse_accumulator: i32,
    pub precompensation: u32,
    shift_word: u32,
    special_generator_state: bool,
    pub enable_non_flux_reversal_generator: bool,
    pub enable_weak_bit_generator: bool,
}

// Write Precompensation is inspired by
// https://github.com/keirf/greaseweazle/blob/master/src/greaseweazle/track.py#L41
// Cache and Memory Hierarchy Design: A Performance-directed Approach. Morgan Kaufmann. pp. 644–. ISBN 978-1-55860-136-9.

impl<T> FluxPulseGenerator<T>
where
    T: FnMut(PulseDuration),
{
    pub fn new(sink: T, cell_duration: u32) -> FluxPulseGenerator<T> {
        FluxPulseGenerator {
            sink,
            cell_duration,
            pulse_accumulator: cell_duration as i32 * -5,
            precompensation: 0,
            shift_word: 0,
            special_generator_state: false,
            enable_non_flux_reversal_generator: false,
            enable_weak_bit_generator: false,
        }
    }

    pub fn flush(&mut self) {
        self.enable_non_flux_reversal_generator = false;
        self.enable_weak_bit_generator = false;

        if (self.shift_word & 0b11111) != 0 {
            self.feed(Bit(false));
            self.feed(Bit(false));
            self.feed(Bit(false));
            self.feed(Bit(false));
            self.feed(Bit(false));
        }
    }

    pub fn feed(&mut self, cell: Bit) {
        self.pulse_accumulator += self.cell_duration as i32;

        // collect incoming cells for later analysis.
        self.shift_word <<= 1;
        if cell.0 {
            self.shift_word |= 1
        }

        if self.special_generator_state {
            if self.enable_weak_bit_generator {
                let weak_cell_len = (self.cell_duration * 2 + self.cell_duration / 2) as i32;
                if self.pulse_accumulator >= weak_cell_len {
                    (self.sink)(PulseDuration(weak_cell_len));
                    self.pulse_accumulator -= weak_cell_len;
                }

                // End the state in expectation of following data
                if (self.shift_word & 0b0001_1000) != 0 {
                    self.special_generator_state = false;
                }
            } else if self.enable_non_flux_reversal_generator {
                (self.sink)(PulseDuration(self.pulse_accumulator));
                self.pulse_accumulator = 0;

                // If we need to have a flux reversal here, make this reversal the one
                // and change back to normal
                if (self.shift_word & 0b0010_0000) != 0 {
                    self.special_generator_state = false;
                }
            }
        }
        // with a window of 5 bitcells we can now perform write precompensation
        // we have 1 cell now, 2 cells in the past and 2 in the future.
        // use the center one as the current
        else if (self.shift_word & 0b0010_0000) != 0 {
            if self.shift_word & 0b011111 == 0
                && (self.enable_weak_bit_generator | self.enable_non_flux_reversal_generator)
            {
                self.special_generator_state = true;
            }

            let next_pulse_accu = match (self.shift_word >> 3) & 0b11111 {
                // there is a very close one in the future. delay the current one.
                0b00101 => {
                    self.pulse_accumulator += self.precompensation as i32;
                    -(self.precompensation as i32)
                }
                // there was a very close one in the past. make this one earlier
                0b10100 => {
                    self.pulse_accumulator -= self.precompensation as i32;
                    self.precompensation as i32
                }
                _ => 0,
            };

            // give a pulse to our sink
            (self.sink)(PulseDuration(self.pulse_accumulator as i32));

            // apply correction onto the accumulator in the opposite direction to avoid phase changes
            // for the next pulse.
            self.pulse_accumulator = next_pulse_accu;
        }
    }
}

pub struct FluxPulseToCells<T>
where
    T: FnMut(Bit),
{
    sink: T,
    pub cell_duration: i32,
}

impl<T> FluxPulseToCells<T>
where
    T: FnMut(Bit),
{
    pub fn new(sink: T, cell_duration: i32) -> FluxPulseToCells<T> {
        FluxPulseToCells {
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

#[cfg(test)]
mod tests {
    use crate::bitstream::{to_bit_stream, BitStreamCollector};

    use super::*;

    #[test]
    fn weak_bits_area_test() {
        let expected_actual_data_on_disk: Vec<u8> = vec![
            0b01010100, //
            0b10000000, 0b00000000, 0b00000001, //
            0b01010001, //
        ];

        let mut normal_data = Vec::new();
        let mut pulse_generator = FluxPulseGenerator::new(|f| normal_data.push(f.0), 100);
        //pulse_generator.weak_bit_generator = true;
        expected_actual_data_on_disk
            .iter()
            .for_each(|f| to_bit_stream(*f, |g| pulse_generator.feed(g)));
        pulse_generator.flush();
        let normal_data_duration: i32 = normal_data.iter().sum();

        let mut weak_bit_data = Vec::new();
        let mut pulse_generator = FluxPulseGenerator::new(|f| weak_bit_data.push(f.0), 100);
        pulse_generator.enable_weak_bit_generator = true;
        expected_actual_data_on_disk
            .iter()
            .for_each(|f| to_bit_stream(*f, |g| pulse_generator.feed(g)));
        pulse_generator.flush();
        let weak_bit_data_duration: i32 = weak_bit_data.iter().sum();

        println!("{} {:?}", normal_data_duration, normal_data);
        println!("{} {:?}", weak_bit_data_duration, weak_bit_data);

        assert_eq!(normal_data, vec![200, 200, 200, 300, 2300, 200, 200, 400]);
        assert_eq!(
            weak_bit_data,
            vec![200, 200, 200, 300, 250, 250, 250, 250, 250, 250, 250, 250, 300, 200, 200, 400]
        );
        assert_eq!(normal_data_duration, weak_bit_data_duration);
    }

    #[test]
    fn non_flux_reversal_area_test() {
        let expected_write_data: Vec<u8> = vec![
            0b01010101, //
            0b01010101, //
            0b01010101, 0b01000100, 0b10001010, //
            0b11111111, 0b11111111, 0b11111111, //
            0b01010001, //
            0b00010101, //
        ];

        let expected_actual_data_on_disk: Vec<u8> = vec![
            0b01010101, //
            0b01010101, //
            0b01010101, 0b01000100, 0b10001010, //
            0b10000000, 0b00000000, 0b00000001, //
            0b01010001, //
            0b00010101, //
        ];

        let mut write_data = Vec::new();
        let mut collector = BitStreamCollector::new(|f| write_data.push(f));
        let mut pulseparser = FluxPulseToCells {
            sink: |val| collector.feed(val),
            cell_duration: 100,
        };
        let mut pulse_generator = FluxPulseGenerator::new(|f| pulseparser.feed(f), 100);
        pulse_generator.enable_non_flux_reversal_generator = true;

        expected_actual_data_on_disk
            .iter()
            .for_each(|f| to_bit_stream(*f, |g| pulse_generator.feed(g)));
        pulse_generator.flush();

        write_data
            .iter()
            .zip(expected_write_data.iter())
            .for_each(|f| println!("{:08b} {:08b}", f.0, f.1));

        assert_eq!(write_data, expected_write_data);
    }

    #[test]
    fn cell_to_pulses_wprecomp_test() {
        let v1: Vec<u8> = vec![
            1, 0, 0, 1, // 3
            0, 0, 1, // 3
            0, 0, 1, // 3
            0, 1, // 2
            0, 1, // 2
            0, 0, 1, // 3
            0, 1, // 2
            0, 1, // 2
            0, 1, // 2
            0, 0, 1, // 3
            0, 0, 1, // 3
            0, 0, 1, // 3
            0, 0, 1, // 3
            0, 1, // 2
            0, 0, 1, // 3
            0, 0, 1, // 3
        ];
        {
            let mut result: Vec<_> = Vec::new();
            let mut pulse_generator = FluxPulseGenerator::new(|f| result.push(f.0), 100);
            v1.iter()
                .for_each(|cell| pulse_generator.feed(Bit(*cell == 1)));
            pulse_generator.flush();

            println!("{:?}", result);
            assert_eq!(
                result,
                vec![
                    100, 300, 300, 300, 200, 200, 300, 200, 200, 200, 300, 300, 300, 300, 200, 300,
                    300
                ]
            );
        }
        {
            let mut result: Vec<_> = Vec::new();
            let mut pulse_generator = FluxPulseGenerator::new(|f| result.push(f.0), 100);
            pulse_generator.precompensation = 10;
            v1.iter()
                .for_each(|cell| pulse_generator.feed(Bit(*cell == 1)));
            pulse_generator.flush();

            let cellsize = 100;
            let compensation = 10;
            println!("{:?}", result);
            assert_eq!(
                result,
                vec![
                    100,
                    cellsize * 3,
                    cellsize * 3,
                    cellsize * 3 + compensation,
                    cellsize * 2 - compensation,
                    cellsize * 2 - compensation,
                    cellsize * 3 + compensation * 2,
                    cellsize * 2 - compensation,
                    cellsize * 2,
                    cellsize * 2 - compensation,
                    cellsize * 3 + compensation,
                    cellsize * 3,
                    cellsize * 3,
                    cellsize * 3 + compensation,
                    cellsize * 2 - compensation * 2,
                    cellsize * 3 + compensation,
                    cellsize * 3,
                ]
            );
        }
    }

    #[test]
    fn cell_to_pulses_test() {
        let v1: Vec<u8> = vec![1, 0, 0, 1, 0, 1, 1, 0, 0, 0, 0, 1];
        let mut result: Vec<PulseDuration> = Vec::new();
        let mut pulse_generator = FluxPulseGenerator::new(|f| result.push(f), 100);
        pulse_generator.enable_non_flux_reversal_generator = false;
        v1.into_iter()
            .for_each(|pulse_duration| pulse_generator.feed(Bit(pulse_duration == 1)));
        pulse_generator.flush();
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
    fn pulse_to_cell_test() {
        let range: Vec<i32> = vec![-49, -20, 0, 20, 49];

        for offset in range {
            let v1 = vec![
                PulseDuration(300 + offset),
                PulseDuration(200 + offset),
                PulseDuration(100 + offset),
                PulseDuration(500 + offset),
            ];

            let mut result: Vec<u32> = Vec::new();

            //result.
            let mut pulseparser = FluxPulseToCells {
                sink: |val| result.push(if val.0 { 1 } else { 0 }),
                cell_duration: 100,
            };
            v1.into_iter().for_each(|f| pulseparser.feed(f));

            println!("{:?}", result);
            assert_eq!(result, vec![0, 0, 1, 0, 1, 1, 0, 0, 0, 0, 1]);
        }
    }
}
