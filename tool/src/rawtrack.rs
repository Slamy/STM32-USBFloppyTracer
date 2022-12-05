use core::panic;
use std::cell::RefCell;

use util::{
    bitstream::to_bit_stream, fluxpulse::FluxPulseGenerator, Bit, DensityMapEntry, Encoding,
    PulseDuration, RawCellData,
};

pub struct RawTrack {
    pub cylinder: u32,
    pub head: u32,
    pub raw_data: Vec<u8>,
    pub densitymap: Vec<DensityMapEntry>,
    pub first_significane_offset: Option<usize>,
    pub encoding: Encoding,
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
                if f.0 > maximum_allowed_cell_size || f.0 < minimum_allowed_cell_size {
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
