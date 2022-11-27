#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod bitstream;
pub mod fluxpulse;
pub mod mfm;

use alloc::vec::Vec;
use ouroboros::self_referencing;

#[derive(Clone, Copy, Debug)]
pub struct Bit(pub bool);

#[derive(Clone, Copy, Debug)]
pub struct Head(pub u8);
#[derive(Clone, Copy, Debug)]
pub struct Cylinder(pub u8);

pub enum DriveSelectState {
    None,
    A,
    B,
}

pub enum Density {
    High,
    SingleDouble,
}

#[derive(Clone, Copy, Debug)]
pub struct Track {
    pub cylinder: Cylinder,
    pub head: Head,
}

pub struct RawCellPart<'a> {
    pub cell_size: PulseDuration,
    pub cells: &'a [u8],
}

#[derive(Clone)]
pub struct DensityMapEntry {
    pub number_of_cells: usize,
    pub cell_size: PulseDuration,
}

#[self_referencing]
pub struct RawCellData {
    pub speeds: Vec<DensityMapEntry>,
    pub cells: Vec<u8>,

    #[borrows(cells)]
    #[covariant]
    //pub cell_sizes: Vec<(u32, PulseDuration)>,
    pub parts: Vec<RawCellPart<'this>>,
}

impl RawCellData {
    pub fn construct(speeds: Vec<DensityMapEntry>, cells: Vec<u8>) -> RawCellData {
        let speeds2 = speeds.clone();

        RawCellDataBuilder {
            speeds,
            cells,

            // Note that the name of the field in the builder
            // is the name of the field in the struct + `_builder`
            // ie: {field_name}_builder
            // the closure that assigns the value for the field will be passed
            // a reference to the field(s) defined in the #[borrows] macro
            parts_builder: |cells| {
                let mut parts: Vec<RawCellPart> = Vec::new();

                let mut offset = 0;
                for speed in speeds2.iter() {
                    let entry = RawCellPart {
                        cell_size: speed.cell_size,
                        cells: &cells[offset..speed.number_of_cells + offset],
                    };
                    parts.push(entry);

                    offset += speed.number_of_cells;
                }

                //x.push(speeds.)
                parts
            },
        }
        .build()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PulseDuration(pub u16);

impl PartialEq<bool> for Bit {
    fn eq(&self, other: &bool) -> bool {
        self.0 == *other
    }
}

impl PulseDuration {
    pub fn similar(&self, other: &PulseDuration) -> bool {
        // TODO Remove magic number
        i16::abs(self.0 as i16 - other.0 as i16) < 60
    }
}

pub use mfm::*;

#[cfg(test)]
mod tests {
    use crc16;
    use std::fs::File;
    use std::{boxed::Box, env};

    use crate::{
        fluxpulse::{FluxPulseGenerator, FluxPulseToCells},
        mfm::MfmDecoder,
    };

    use super::*;

    struct Unreliable<T> {
        pub feeder: Box<dyn Iterator<Item = T>>,
        give_none: bool,
    }

    impl<T> Iterator for Unreliable<T> {
        type Item = T;
        fn next(&mut self) -> Option<Self::Item> {
            self.give_none = !self.give_none;
            if self.give_none {
                self.feeder.next()
            } else {
                None
            }
        }
    }

    #[test]
    fn pulsegenerator_test() {
        let v1: Vec<u8> = vec![0, 0, 0, 1, 1, 0, 1, 1];
        let bit_source = Box::new(v1.into_iter().map(|x| Bit(x == 1)));
        let encoder = MfmEncoder::new(bit_source);
        let pulse_generator = FluxPulseGenerator::new(Box::new(encoder), 100);

        let result: Vec<PulseDuration> = pulse_generator.collect();

        println!("{:?}", result);
        assert_eq!(
            result,
            vec![
                PulseDuration(200),
                PulseDuration(200),
                PulseDuration(300),
                PulseDuration(200),
                PulseDuration(400),
                PulseDuration(200)
            ]
        );
    }

    fn read_saleae_pulses(path: &str) -> Vec<PulseDuration> {
        let file = File::open(path).unwrap();
        let mut reader = csv::Reader::from_reader(file);

        let mut last: f32 = 0.0;

        reader
            .records()
            .map(|d| {
                let x = d.unwrap();
                (x[0].parse::<f32>().unwrap(), x[1].parse::<i32>().unwrap())
            })
            .filter(|d| d.1 == 1)
            .map(|f| {
                let result = f.0 - last;
                last = f.0;
                PulseDuration((result * 84000000.0).round() as u16)
            })
            .collect()
    }

    #[test]
    fn hd_dos_idam_test() {
        let logic_result = read_saleae_pulses("../saleae/hd_msdos_onlyIdam.csv").into_iter();
        let pulsed = FluxPulseToCells::new(Box::new(logic_result), 84);

        let mfmd = MfmDecoder::new(Box::new(pulsed));
        let result: Vec<mfm::MfmResult> = mfmd
            .filter(|f| matches!(f, mfm::MfmResult::Got(_)))
            .collect();
        assert_eq!(
            result,
            vec![
                mfm::MfmResult::Got(254),
                mfm::MfmResult::Got(7),
                mfm::MfmResult::Got(0),
                mfm::MfmResult::Got(1),
                mfm::MfmResult::Got(2)
            ]
        );
    }

    #[test]
    fn hd_dos_track_test() {
        let logic_result = read_saleae_pulses("../saleae/hd_msdos_track.csv").into_iter();
        let pulsed = FluxPulseToCells::new(Box::new(logic_result), 84);
        let mfmd = MfmDecoder::new(Box::new(pulsed));
        let result: Vec<mfm::MfmResult> = mfmd.collect();

        let mut in_sync = 0;
        let mut buffer = Vec::new();
        let mut crc = crc16::State::<crc16::CCITT_FALSE>::new();

        for p in result {
            match p {
                mfm::MfmResult::Got(x) => {
                    //crc.update(&x.to_ne_bytes());
                    buffer.push(x);

                    in_sync -= 1;
                    if in_sync == 0 && buffer[0] == 0xfe {
                        crc.update(&buffer);
                        println!("{:02x?} CRC {}", buffer, crc.get());
                        assert!(crc.get() == 0);
                        assert!(buffer[4] == 2); // 512 byte sector
                    }
                }
                mfm::MfmResult::Pending => { /*ignore */ }
                mfm::MfmResult::SyncWord => {
                    in_sync = 7;
                    //println!("Sync!");
                    buffer = Vec::new();
                    crc = crc16::State::<crc16::CCITT_FALSE>::new();
                    crc.update(&vec![0xa1, 0xa1, 0xa1]);
                }
                mfm::MfmResult::Searching => { /*ignore */ }
            };
            // println!("{:?}", p);
        }
    }
}
