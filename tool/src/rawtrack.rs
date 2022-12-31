use core::panic;
use std::{cell::RefCell, collections::VecDeque};

use util::{
    bitstream::to_bit_stream,
    fluxpulse::FluxPulseGenerator,
    mfm::{MfmDecoder, MfmWord},
    Bit, Density, DensityMap, DiskType, Encoding, PulseDuration, RawCellData, STM_TIMER_MHZ,
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
                f.head <= head
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
    pub first_significane_offset: Option<usize>,
    pub encoding: Encoding,
    pub write_precompensation: u32,
    pub has_non_flux_reversal_area: bool,
}

impl RawTrack {
    pub fn new(
        cylinder: u32,
        head: u32,
        raw_data: Vec<u8>,
        densitymap: DensityMap,
        encoding: Encoding,
    ) -> Self {
        RawTrack {
            cylinder,
            head,
            raw_data,
            densitymap,
            first_significane_offset: None,
            encoding,
            write_precompensation: 0,
            has_non_flux_reversal_area: false,
        }
    }

    pub fn new_with_non_flux_reversal_area(
        cylinder: u32,
        head: u32,
        raw_data: Vec<u8>,
        densitymap: DensityMap,
        encoding: Encoding,
        has_non_flux_reversal_area: bool,
    ) -> Self {
        RawTrack {
            cylinder,
            head,
            raw_data,
            densitymap,
            first_significane_offset: None,
            encoding,
            write_precompensation: 0,
            has_non_flux_reversal_area,
        }
    }

    fn find_significance_longer_pulses(
        &self,
        pulses: &[PulseDuration],
        threshold: PulseDuration,
    ) -> Option<usize> {
        let mut significance = 0;

        let pulses_iter = pulses.iter();

        for (i, val) in pulses_iter.enumerate() {
            if val.0 > threshold.0 {
                significance += 2;

                // TODO magic number
                if significance >= 4 {
                    return Some(i);
                }
            } else if significance > 0 {
                significance -= 1;
            }
        }
        None
    }

    // TODO documentation
    fn find_significance_through_divergence(
        &self,
        pulses: &[PulseDuration],
        reference: PulseDuration,
    ) -> Option<usize> {
        let mut significance = 0;

        let pulses_iter = pulses.iter();

        for (i, val) in pulses_iter.enumerate() {
            if val.0 != reference.0 {
                significance += 2;

                // TODO magic number
                if significance >= 8 {
                    // TODO magic number
                    if i < 8 {
                        return None;
                    } else {
                        return Some(i);
                    }
                }
            } else if significance > 0 {
                significance -= 1;
            }
        }
        None
    }

    pub fn get_significance_offset(&mut self) -> usize {
        // TODO this is ugly
        let pulses = self.convert_to_pulses();

        let mut possible_offset = self.find_significance_through_divergence(&pulses, pulses[0]);
        if let Some(offset) = possible_offset {
            self.first_significane_offset = possible_offset;
            return offset;
        }

        // TODO remove magic numbers
        possible_offset =
            self.find_significance_longer_pulses(&pulses, PulseDuration(168 * 2 + 30));
        if let Some(offset) = possible_offset {
            self.first_significane_offset = possible_offset;
            return offset;
        }

        // TODO this is ugly too
        panic!(
            "Unable to find an offset of significance for the verification of track {}!",
            self.cylinder
        );
    }

    fn convert_to_pulses(&self) -> Vec<PulseDuration> {
        let mut result = Vec::new();

        // TODO avoid clone
        let cell_data =
            RawCellData::construct(self.densitymap.clone(), self.raw_data.clone(), false);
        let mut write_prod_fpg = FluxPulseGenerator::new(|f| result.push(f), 0);

        // start with a flux transition. avoids long sequences of zero
        for part in cell_data.borrow_parts() {
            write_prod_fpg.cell_duration = part.cell_size.0 as u32;

            for cell_byte in part.cells {
                to_bit_stream(*cell_byte, |bit| write_prod_fpg.feed(bit));
            }
        }
        write_prod_fpg.feed(Bit(true));

        result
    }

    pub fn calculate_duration_of_track(&self) -> f64 {
        let mut accumulator = 0.0;

        for entry in self.densitymap.iter() {
            let seconds_per_cell: f64 = 1e-6_f64 * entry.cell_size.0 as f64 / STM_TIMER_MHZ;
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

    pub fn check_writability(&self) {
        let minimum_allowed_cell_size = match self.encoding {
            util::Encoding::GCR => {
                // Abort this for GCR as currently every GCR stream is writable
                // If pauses are too long, they will be filled up with weak bits.
                // Pauses can't be too short for GCR as we are working with full cells
                return;
            }
            // With MFM this is a different story as we are working with half cells.
            // The drive mechanism expects us to have at least one half cell pause
            // between the flux reversals. If this rule is not applied here,
            // the data we read bacl will be different.
            util::Encoding::MFM => self.densitymap[0].cell_size.0 + 40,
        };

        // TODO avoid the clone
        let cell_data =
            RawCellData::construct(self.densitymap.clone(), self.raw_data.clone(), false);
        let track_offset = RefCell::new(0);

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
                        (current_track_offset - 5) as usize
                    };

                    let impossible_data_position =
                        &self.raw_data[start_view..current_track_offset + 5];
                    println!("impossible_data_position {:x?}", impossible_data_position);

                    for i in impossible_data_position.iter() {
                        println!("{:02x} {:08b}", i, i);
                    }

                    let zero_pos = self.raw_data.iter().position(|d| *d == 0);
                    if let Some(zero_found) = zero_pos {
                        println!("zero_found at {}. This track needs fixing.", zero_found);
                        println!("zero to end is {}", self.raw_data.len() - zero_found);
                    }

                    panic!("Too short pause between flux change: {}", f.0)
                }
            },
            self.densitymap[0].cell_size.0 as u32,
        );

        if matches!(self.encoding, util::Encoding::MFM) {
            write_prod_fpg.feed(Bit(false));
        }

        for part in cell_data.borrow_parts() {
            write_prod_fpg.cell_duration = part.cell_size.0 as u32;

            for cell_byte in part.cells {
                *track_offset.borrow_mut() += 1;
                to_bit_stream(*cell_byte, |bit| write_prod_fpg.feed(bit));
            }
        }
    }
}

pub fn auto_cell_size(tracklen: u32, rpm: f64) -> f64 {
    let number_cells = tracklen * 8;
    let seconds_per_revolution = 60.0 / rpm;
    let microseconds_per_cell = 10_f64.powi(6) * seconds_per_revolution / number_cells as f64;
    let raw_timer_val = STM_TIMER_MHZ * microseconds_per_cell;
    raw_timer_val
}

pub fn print_iso_sector_data(trackdata: &[u8], idam_sector: u8) {
    let queue = RefCell::new(VecDeque::new());
    let mut mfmd = MfmDecoder::new(|f| queue.borrow_mut().push_front(f));

    let mut data_iter = trackdata.iter();

    let mut awaiting_dam = 0;
    let mut sector_header = Vec::new();

    loop {
        while queue.borrow().len() < 3 {
            to_bit_stream(*data_iter.next().unwrap(), |f| mfmd.feed(f));
        }

        awaiting_dam -= 1;

        let mfm = queue.borrow_mut().pop_back().unwrap();

        if matches!(mfm, MfmWord::SyncWord) {
            let sync_type = queue.borrow_mut().pop_back().unwrap();
            println!("{} {:x?}", awaiting_dam, sync_type);

            if awaiting_dam > 0 && matches!(sync_type, MfmWord::Enc(0xfb)) {
                println!("We got our data!");
                break;
            }

            if !matches!(sync_type, MfmWord::Enc(0xfe)) {
                continue;
            }

            // Well we go a Sector Header. Now read and process it!
            while queue.borrow().len() < 8 {
                to_bit_stream(*data_iter.next().unwrap(), |f| mfmd.feed(f));
            }

            // Sector header
            sector_header.clear();

            for _ in 0..6 {
                if let MfmWord::Enc(val) = queue.borrow_mut().pop_back().unwrap() {
                    sector_header.push(val);
                }
            }
            println!("{:x?}", sector_header);

            if sector_header[2] != idam_sector {
                continue;
            }

            // Ok this is our sector!
            let mut crc = crc16::State::<crc16::CCITT_FALSE>::new();
            crc.update(&vec![0xa1, 0xa1, 0xa1, 0xfe]);
            crc.update(&sector_header);
            let crc16 = crc.get();
            assert_eq!(crc16, 0);

            // CRC is fine!
            awaiting_dam = 40;
            println!("This is our header!");
        }
    }

    println!("{:x?}", sector_header);
    let sector_size = 128 << sector_header[3];
    let mut sector_data = Vec::new();
    mfmd.sync_detector_active = false;

    while queue.borrow().len() < sector_size {
        to_bit_stream(*data_iter.next().unwrap(), |f| mfmd.feed(f));
    }

    for _ in 0..sector_size {
        let MfmWord::Enc(value) = queue.borrow_mut().pop_back().unwrap() else {panic!();};
        sector_data.push(value);
    }
    println!("{:x?}", sector_data);
}

pub struct TrackFilter {
    cyl_start: Option<u32>,
    cyl_end: Option<u32>,
    head: Option<u32>,
}
impl TrackFilter {
    fn from_track_split(track_split: Vec<&str>, head: Option<u32>) -> Self {
        if track_split.len() == 1 {
            return TrackFilter {
                cyl_start: track_split[0].parse().ok(),
                cyl_end: track_split[0].parse().ok(),
                head,
            };
        } else if track_split.len() == 2 {
            return TrackFilter {
                cyl_start: track_split[0].parse().ok(),
                cyl_end: track_split[1].parse().ok(),
                head,
            };
        }
        panic!("Unexpected track filter parameter!")
    }

    pub fn new(param: &str) -> Self {
        let head_split: Vec<_> = param.split(":").collect();
        let track_split: Vec<&str> = head_split[0].split("-").collect();

        if head_split.len() == 1 {
            return Self::from_track_split(track_split, None);
        } else if head_split.len() == 2 {
            return Self::from_track_split(track_split, head_split[1].parse().ok());
        }
        panic!("Unexpected track filter parameter!")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_filter_test() {
        let filter = TrackFilter::new("2-10");
        assert_eq!(filter.cyl_end.unwrap(), 10);
        assert_eq!(filter.cyl_start.unwrap(), 2);
        assert!(filter.head.is_none());

        let filter = TrackFilter::new("2-");
        assert!(filter.cyl_end.is_none());
        assert_eq!(filter.cyl_start.unwrap(), 2);
        assert!(filter.head.is_none());

        let filter = TrackFilter::new("-8");
        assert!(filter.cyl_start.is_none());
        assert_eq!(filter.cyl_end.unwrap(), 8);
        assert!(filter.head.is_none());

        let filter = TrackFilter::new("2-10:1");
        assert_eq!(filter.cyl_end.unwrap(), 10);
        assert_eq!(filter.cyl_start.unwrap(), 2);
        assert_eq!(filter.head.unwrap(), 1);

        let filter = TrackFilter::new("2-8:0");
        assert_eq!(filter.cyl_end.unwrap(), 8);
        assert_eq!(filter.cyl_start.unwrap(), 2);
        assert_eq!(filter.head.unwrap(), 0);

        let filter = TrackFilter::new("34");
        assert_eq!(filter.cyl_end.unwrap(), 34);
        assert_eq!(filter.cyl_start.unwrap(), 34);
        assert!(filter.head.is_none());
    }
}
