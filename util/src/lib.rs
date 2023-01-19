#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod bitstream;
pub mod c64_geometry;
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
#[derive(Clone, Copy, Debug)]
pub enum DiskType {
    Inch3_5,
    Inch5_25,
}

pub enum DriveSelectState {
    None,
    A,
    B,
}
#[derive(Clone, Copy, Debug)]
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

#[derive(Clone, Debug)]
pub struct DensityMapEntry {
    pub number_of_cellbytes: usize,
    pub cell_size: PulseDuration,
}

pub const DRIVE_5_25_RPM: f64 = 361.0; // Normally 360 RPM would be correct. But the drive might be faster. Let's be safe here.
pub const DRIVE_3_5_RPM: f64 = 300.2; // Normally 300 RPM would be correct. But the drive might be faster. Let's be safe here.
pub const STM_TIMER_MHZ: f64 = 84.0;
pub const STM_TIMER_HZ: f64 = 84e6;

pub fn duration_of_rotation_as_stm_tim_raw(rpm: f64) -> usize {
    (60.0 / rpm * STM_TIMER_HZ) as usize
}

pub type DensityMap = Vec<DensityMapEntry>;

pub fn reduce_densitymap(densitymap: DensityMap) -> DensityMap {
    let mut result: DensityMap = Vec::new();

    for entry in densitymap {
        if let Some(last) = result.last_mut() && entry.cell_size == last.cell_size {
            // use the current one
            last.number_of_cellbytes+=entry.number_of_cellbytes;
        }
        else{
            result.push(entry);
        }
    }
    result
}
#[self_referencing]
pub struct RawCellData {
    pub speeds: DensityMap,
    pub cells: Vec<u8>,
    pub has_non_flux_reversal_area: bool,
    #[borrows(cells)]
    #[covariant]
    pub parts: Vec<RawCellPart<'this>>,
}

impl RawCellData {
    pub fn construct(speeds: DensityMap, cells: Vec<u8>, has_non_flux_reversal_area: bool) -> Self {
        let speeds2 = speeds.clone();

        RawCellDataBuilder {
            speeds,
            cells,
            has_non_flux_reversal_area,

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
                        cells: &cells[offset..speed.number_of_cellbytes + offset],
                    };
                    parts.push(entry);

                    offset += speed.number_of_cellbytes;
                }

                // just to be sure that the separate parts in sum are equal to the total number
                assert_eq!(offset, cells.len());

                parts
            },
        }
        .build()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PulseDuration(pub i32);

impl PartialEq<bool> for Bit {
    fn eq(&self, other: &bool) -> bool {
        self.0 == *other
    }
}

impl PulseDuration {
    pub fn similar(&self, other: &PulseDuration, threshold: i32) -> bool {
        i32::abs(self.0 - other.0) < threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_of_rotation_as_stm_tim_raw_test() {
        let result = duration_of_rotation_as_stm_tim_raw(300.0);
        assert_eq!(result as u32, 16800000);
    }
}
