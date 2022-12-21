use core::panic;
use std::{cell::RefCell, collections::VecDeque};

use util::{
    bitstream::to_bit_stream,
    fluxpulse::FluxPulseGenerator,
    mfm::{MfmDecoder, MfmWord},
    Bit, Density, DensityMapEntry, DiskType, Encoding, PulseDuration, RawCellData,
};

pub struct RawImage {
    pub density: Density,
    pub disk_type: DiskType,
    pub tracks: Vec<RawTrack>,
}
pub struct RawTrack {
    pub cylinder: u32,
    pub head: u32,
    pub raw_data: Vec<u8>,
    pub densitymap: Vec<DensityMapEntry>,
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
        densitymap: Vec<DensityMapEntry>,
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
        densitymap: Vec<DensityMapEntry>,
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
            println!(
                "Divergence Significance for track {} {} at {}",
                self.cylinder, self.head, offset
            );

            self.first_significane_offset = possible_offset;
            return offset;
        }

        // TODO remove magic numbers
        possible_offset =
            self.find_significance_longer_pulses(&pulses, PulseDuration(168 * 2 + 30));
        if let Some(offset) = possible_offset {
            println!(
                "Longer pulses Significance for track {} {} at {}",
                self.cylinder, self.head, offset
            );

            self.first_significane_offset = possible_offset;
            return offset;
        }

        // TODO this is ugly too
        panic!();
    }

    fn convert_to_pulses(&self) -> Vec<PulseDuration> {
        let mut result = Vec::new();

        // TODO avoid clone
        let cell_data = RawCellData::construct(self.densitymap.clone(), self.raw_data.clone());
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

    pub fn check_writability(&self) {
        // TODO avoid the clone
        let cell_data = RawCellData::construct(self.densitymap.clone(), self.raw_data.clone());

        let maximum_allowed_cell_size = match self.encoding {
            util::Encoding::GCR => self.densitymap[0].cell_size.0 * 5,
            util::Encoding::MFM => self.densitymap[0].cell_size.0 * 8,
        };

        let minimum_allowed_cell_size = match self.encoding {
            util::Encoding::GCR => self.densitymap[0].cell_size.0 - 40,
            util::Encoding::MFM => self.densitymap[0].cell_size.0 + 40,
        };

        let track_offset = RefCell::new(0);

        let mut write_prod_fpg = FluxPulseGenerator::new(
            |f| {
                if f.0 > maximum_allowed_cell_size && self.has_non_flux_reversal_area {
                    println!(
                        "INFO: Track {} {} has a non flux reversal area...",
                        self.cylinder, self.head
                    );
                } else if f.0 > maximum_allowed_cell_size || f.0 < minimum_allowed_cell_size {
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

                    let zero_pos = self.raw_data.iter().position(|d| *d == 0);
                    if let Some(zero_found) = zero_pos {
                        println!("zero_found at {}. This track needs fixing.", zero_found);
                        println!("zero to end is {}", self.raw_data.len() - zero_found);
                    }

                    panic!("Too long pause between flux change: {}", f.0)
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

pub const DRIVE_5_25_RPM: f64 = 361.0; // Normally 360 RPM would be correct. But the drive might be faster. Let's be safe here.
pub const DRIVE_3_5_RPM: f64 = 300.2; // Normally 300 RPM would be correct. But the drive might be faster. Let's be safe here.

pub fn auto_cell_size(tracklen: u32, rpm: f64) -> f64 {
    let number_cells = tracklen * 8;
    let seconds_per_revolution = 60.0 / rpm;
    let microseconds_per_cell = 10_f64.powi(6) * seconds_per_revolution / number_cells as f64;
    let stm_timer_mhz = 84.0;
    let raw_timer_val = stm_timer_mhz * microseconds_per_cell;
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
