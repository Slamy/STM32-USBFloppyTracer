use core::panic;
use std::cell::RefCell;

use anyhow::Context;
use util::{
    bitstream::to_bit_stream, fluxpulse::FluxPulseGenerator, Bit, Density, DensityMap, DiskType,
    Encoding, RawCellData, STM_TIMER_MHZ,
};

pub struct RawImage {
    pub density: Density,
    pub disk_type: DiskType,
    pub tracks: Vec<RawTrack>,
}

impl RawImage {
    pub fn filter_tracks(&mut self, filter: TrackFilter) {
        self.tracks.retain(|f| {
            (if let Some(cyl_start) = filter.cyl_start {
                f.cylinder >= cyl_start
            } else {
                true
            }) && (if let Some(cyl_end) = filter.cyl_end {
                f.cylinder <= cyl_end
            } else {
                true
            }) && (if let Some(head) = filter.head {
                f.head == head
            } else {
                true
            })
        });
    }
}

pub struct RawTrack {
    pub cylinder: u32,
    pub head: u32,
    pub raw_data: Vec<u8>,
    pub densitymap: DensityMap,
    pub encoding: Encoding,
    pub write_precompensation: u32,
    pub has_non_flux_reversal_area: bool,
}

impl RawTrack {
    #[must_use]
    pub fn new(
        cylinder: u32,
        head: u32,
        raw_data: Vec<u8>,
        densitymap: DensityMap,
        encoding: Encoding,
    ) -> Self {
        Self {
            cylinder,
            head,
            raw_data,
            densitymap,
            encoding,
            write_precompensation: 0,
            has_non_flux_reversal_area: false,
        }
    }

    #[must_use]
    pub fn new_with_non_flux_reversal_area(
        cylinder: u32,
        head: u32,
        raw_data: Vec<u8>,
        densitymap: DensityMap,
        encoding: Encoding,
        has_non_flux_reversal_area: bool,
    ) -> Self {
        Self {
            cylinder,
            head,
            raw_data,
            densitymap,
            encoding,
            write_precompensation: 0,
            has_non_flux_reversal_area,
        }
    }

    #[must_use]
    pub fn calculate_duration_of_track(&self) -> f64 {
        let mut accumulator = 0.0;

        for entry in &self.densitymap {
            let seconds_per_cell: f64 = 1e-6_f64 * f64::from(entry.cell_size.0) / STM_TIMER_MHZ;
            accumulator += seconds_per_cell * entry.number_of_cellbytes as f64 * 8.0;
        }

        accumulator
    }

    pub fn assert_fits_into_rotation(&self, rpm: f64) {
        let seconds_per_rotation = 60.0 / rpm;
        let duration_of_track = self.calculate_duration_of_track();

        assert!(
            duration_of_track < seconds_per_rotation,
            "Error: With {} seconds, the track {} will not fit into one single rotation of the disk!",
            duration_of_track, self.cylinder
        );
    }

    pub fn check_writability(&self) -> bool {
        let Some(first_cell_size) = self.densitymap.get(0) else {return false;};
        let first_cell_size = first_cell_size.cell_size.0;

        let minimum_allowed_cell_size = match self.encoding {
            util::Encoding::GCR => {
                // Abort this for GCR as currently every GCR stream is writable
                // If pauses are too long, they will be filled up with weak bits.
                // Pauses can't be too short for GCR as we are working with full cells
                return true;
            }
            // With MFM this is a different story as we are working with half cells.
            // The drive mechanism expects us to have at least one half cell pause
            // between the flux reversals. If this rule is not applied here,
            // the data we read bacl will be different.
            util::Encoding::MFM => first_cell_size + 40,
        };

        let cell_data_parts = RawCellData::split_in_parts(&self.densitymap, &self.raw_data);
        let track_offset = RefCell::new(0);
        let mut is_writable = true;

        let mut write_prod_fpg = FluxPulseGenerator::new(
            |f| {
                if f.0 < minimum_allowed_cell_size {
                    let current_track_offset = *track_offset.borrow();

                    println!(
                        "Track {} {} has physically impossible data. Offset {} of {}. Reduce by {}?",
                        self.cylinder,
                        self.head,
                        current_track_offset,
                        self.raw_data.len(),
                        self.raw_data.len() - current_track_offset
                    );

                    let start_view = if current_track_offset < 5 {
                        0
                    } else {
                        current_track_offset - 5
                    };

                    if let Some(impossible_data_position) =
                        self.raw_data.get(start_view..current_track_offset + 5)
                    {
                        println!("impossible_data_position {impossible_data_position:x?}");

                        for i in impossible_data_position.iter() {
                            println!("{i:02x} {i:08b}");
                        }

                        let zero_pos = self.raw_data.iter().position(|d| *d == 0);
                        if let Some(zero_found) = zero_pos {
                            println!("zero_found at {zero_found}. This track needs fixing.");
                            println!("zero to end is {}", self.raw_data.len() - zero_found);
                        }
                    } else {
                        println!("Unable to show position of impossible data");
                    }

                    println!("Too short pause between flux change: {}", f.0);
                    is_writable = false;
                }
            },
            first_cell_size as u32,
        );

        if matches!(self.encoding, util::Encoding::MFM) {
            write_prod_fpg.feed(Bit(false));
        }

        for part in cell_data_parts {
            write_prod_fpg.cell_duration = part.cell_size.0 as u32;

            for cell_byte in part.cells {
                *track_offset.borrow_mut() += 1;
                to_bit_stream(*cell_byte, |bit| write_prod_fpg.feed(bit));
            }
        }

        is_writable
    }
}

#[must_use]
pub fn auto_cell_size(tracklen: u32, rpm: f64) -> f64 {
    let number_cells = tracklen * 8;
    let seconds_per_revolution = 60.0 / rpm;
    let microseconds_per_cell = 10_f64.powi(6) * seconds_per_revolution / f64::from(number_cells);
    STM_TIMER_MHZ * microseconds_per_cell
}

#[derive(Clone, Copy, Debug)]
pub struct TrackFilter {
    pub cyl_start: Option<u32>,
    pub cyl_end: Option<u32>,
    pub head: Option<u32>,
}
impl TrackFilter {
    fn from_track_split(track_split: &[&str], head: Option<u32>) -> anyhow::Result<Self> {
        let mut track_split_iter = track_split.iter();

        if let Some(cyl_start) = track_split_iter.next() {
            let cyl_start = cyl_start.parse().ok();

            let cyl_end = if let Some(cyl_end) = track_split_iter.next() {
                cyl_end.parse().ok()
            } else {
                cyl_start
            };

            Ok(Self {
                cyl_start,
                cyl_end,
                head,
            })
        } else {
            Err(anyhow::anyhow!("Unexpected track filter parameter!"))
        }
    }

    pub fn new(param: &str) -> anyhow::Result<Self> {
        if param.is_empty() || param == "-" {
            return Err(anyhow::anyhow!("Empty parameter!"));
        }

        let head_split: Vec<_> = param.split(':').collect();
        let track_split: Vec<&str> = head_split
            .first()
            .context("Invalid argument")?
            .split('-')
            .collect();

        if head_split.len() == 1 {
            return Self::from_track_split(&track_split, None);
        } else if head_split.len() == 2 {
            return Self::from_track_split(
                &track_split,
                head_split.get(1).context("Wont happen")?.parse().ok(),
            );
        }
        Err(anyhow::anyhow!("Unexpected track filter parameter!"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_filter_test() {
        let filter = TrackFilter::new("2-10").unwrap();
        assert_eq!(filter.cyl_end.unwrap(), 10);
        assert_eq!(filter.cyl_start.unwrap(), 2);
        assert!(filter.head.is_none());

        let filter = TrackFilter::new("2-").unwrap();
        assert!(filter.cyl_end.is_none());
        assert_eq!(filter.cyl_start.unwrap(), 2);
        assert!(filter.head.is_none());

        let filter = TrackFilter::new("-8").unwrap();
        assert!(filter.cyl_start.is_none());
        assert_eq!(filter.cyl_end.unwrap(), 8);
        assert!(filter.head.is_none());

        let filter = TrackFilter::new("2-10:1").unwrap();
        assert_eq!(filter.cyl_end.unwrap(), 10);
        assert_eq!(filter.cyl_start.unwrap(), 2);
        assert_eq!(filter.head.unwrap(), 1);

        let filter = TrackFilter::new("2-8:0").unwrap();
        assert_eq!(filter.cyl_end.unwrap(), 8);
        assert_eq!(filter.cyl_start.unwrap(), 2);
        assert_eq!(filter.head.unwrap(), 0);

        let filter = TrackFilter::new("34").unwrap();
        assert_eq!(filter.cyl_end.unwrap(), 34);
        assert_eq!(filter.cyl_start.unwrap(), 34);
        assert!(filter.head.is_none());

        let filter = TrackFilter::new("");
        assert!(filter.is_err());

        let filter = TrackFilter::new("-");
        assert!(filter.is_err());
    }
}
