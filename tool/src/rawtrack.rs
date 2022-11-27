use core::panic;

use util::{
    bitstream::to_bit_stream, fluxpulse::FluxPulseGenerator2, Bit, DensityMapEntry, PulseDuration,
    RawCellData,
};

pub struct RawTrack {
    pub cylinder: u32,
    pub head: u32,
    pub raw_data: Vec<u8>,
    pub densitymap: Vec<DensityMapEntry>,
    //pulses: Option<Vec<PulseDuration>>,
    pub first_significane_offset: Option<usize>,
}

impl RawTrack {
    pub fn new(
        cylinder: u32,
        head: u32,
        raw_data: Vec<u8>,
        densitymap: Vec<DensityMapEntry>,
    ) -> Self {
        RawTrack {
            cylinder,
            head,
            raw_data,
            densitymap,
            first_significane_offset: None,
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
                if significance >= 8 {
                    // println!("Long Pulse Significance at {}", i);
                    return Some(i);
                }

                // TODO For Z-Out
                if significance >= 4 && i > 15 {
                    // println!("Long Pulse Significance at {}", i);
                    return Some(i);
                }
            } else if significance > 0 {
                significance -= 1;
            }

            //println!("{} {} {}", i, val.0, significance);
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
                    // println!("Divergence Significance at {}", i);

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

            // println!("{} {} {}", i, val.0, significance);
        }
        None
    }

    pub fn get_significance_offset(&mut self) -> usize {
        // TODO this is ugly
        // println!("Track {}", self.cylinder);

        let pulses = self.convert_to_pulses();

        let mut possible_offset = self.find_significance_through_divergence(&pulses, pulses[0]);
        if let Some(offset) = possible_offset {
            /*
            println!(
                "Divergence Significance for track {} {} at {}",
                self.cylinder, self.head, offset
            );
            */
            self.first_significane_offset = possible_offset;
            return offset;
        }

        // TODO remove magic numbers
        possible_offset =
            self.find_significance_longer_pulses(&pulses, PulseDuration(168 * 2 + 30));
        if let Some(offset) = possible_offset {
            /*
            println!(
                "Longer pulses Significance for track {} {} at {}",
                self.cylinder, self.head, offset
            );
            */
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
        let mut write_prod_fpg = FluxPulseGenerator2::new(|f| result.push(f), 0);

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
}
