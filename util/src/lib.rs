#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod bitstream;
pub mod fluxpulse;
pub mod gcr;
pub mod mfm;

use alloc::vec::Vec;
use ouroboros::self_referencing;

#[derive(Clone, Copy, Debug)]
pub struct Bit(pub bool);

#[derive(Clone, Copy, Debug)]
pub struct Head(pub u8);
#[derive(Clone, Copy, Debug)]
pub struct Cylinder(pub u8);

#[derive(Clone, Copy, Debug)]
pub enum Encoding {
    GCR,
    MFM,
}

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
    pub fn similar(&self, other: &PulseDuration, threshold: i16) -> bool {
        i16::abs(self.0 as i16 - other.0 as i16) < threshold
    }
}
