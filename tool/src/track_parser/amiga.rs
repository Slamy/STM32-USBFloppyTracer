use std::convert::TryInto;

use anyhow::ensure;
use util::{
    duration_of_rotation_as_stm_tim_raw,
    fluxpulse::FluxPulseToCells,
    mfm::{MfmDataSeperator, RawMfmWord},
    Density, PulseDuration, DRIVE_3_5_RPM,
};

use crate::{rawtrack::TrackFilter, track_parser::concatenate_sectors};

use super::{CollectedSector, TrackParser, TrackPayload};

const AMIGA_MFM_MASK: u32 = 0x5555_5555;
const WORDS_PER_SECTOR: usize = 128;
pub const SECTORS_PER_AMIGA_DD_TRACK: usize = 11;

fn read_even_bits<'a>(iterator: &mut impl Iterator<Item = &'a RawMfmWord>) -> u32 {
    match iterator.next() {
        Some(RawMfmWord::Raw(raw)) => raw & AMIGA_MFM_MASK,
        _ => 0, // SyncWord as well
    }
}

pub struct AmigaTrackParser {
    collected_sectors: Option<Vec<CollectedSector>>,
    expected_sectors_per_track: usize,
    expected_track_number: Option<u32>,
}

impl AmigaTrackParser {
    #[must_use]
    pub fn new(disk_type: Density) -> Self {
        let expected_sectors_per_track = match disk_type {
            Density::High => 22,
            Density::SingleDouble => 11,
        };

        Self {
            collected_sectors: None,
            expected_sectors_per_track,
            expected_track_number: None,
        }
    }
}

impl TrackParser for AmigaTrackParser {
    fn default_file_extension(&self) -> &str {
        "adf"
    }

    fn duration_to_record(&self) -> usize {
        duration_of_rotation_as_stm_tim_raw(DRIVE_3_5_RPM) * 110 / 100
    }

    fn parse_raw_track(&mut self, track: &[u8]) -> anyhow::Result<TrackPayload> {
        let expected_track_number = self.expected_track_number.expect("Program flow error");
        let cellsize_2micros = 168;
        let mut mfm_words: Vec<RawMfmWord> = Vec::new();
        let mut mfmd = MfmDataSeperator::new(|f| mfm_words.push(f));
        let mut pulseparser = FluxPulseToCells::new(|val| mfmd.feed(val), cellsize_2micros);

        for mfm_word in track {
            pulseparser.feed(PulseDuration(i32::from(*mfm_word) << 3));
        }

        let mut iterator = mfm_words.iter();

        // Search for Syncs until the end.
        while let Some(searchword) = iterator.next() {
            if matches!(searchword, RawMfmWord::SyncWord) {
                // We have found a sync. Let's try to parse the potential upcoming sector
                match parse_amiga_sector(&mut iterator, expected_track_number) {
                    Ok(just_gotten_sector) => {
                        // Did we get this sector yet?
                        let collected_sectors = self.collected_sectors.as_mut().unwrap();

                        if !collected_sectors
                            .iter()
                            .any(|f| f.index == just_gotten_sector.index)
                        {
                            collected_sectors.push(just_gotten_sector);

                            if collected_sectors.len() == self.expected_sectors_per_track {
                                // Exit it after we got all expected sectors.
                                break;
                            }
                        }
                    }
                    Err(_err) => {
                        // Just ignore it.
                    }
                };
            }
        }

        ensure!(self.collected_sectors.as_ref().unwrap().len() == self.expected_sectors_per_track);
        let collected_sectors = self.collected_sectors.take().unwrap();

        Ok(concatenate_sectors(
            collected_sectors,
            expected_track_number >> 1,
            expected_track_number & 1,
        ))
    }

    fn expect_track(&mut self, cylinder: u32, head: u32) {
        self.expected_track_number = Some((cylinder << 1) | head);
        self.collected_sectors = Some(Vec::new());
    }

    fn step_size(&self) -> usize {
        1
    }

    fn track_density(&self) -> Density {
        Density::SingleDouble
    }

    fn format_name(&self) -> &str {
        "AmigaDOS"
    }

    fn default_trackfilter(&self) -> crate::rawtrack::TrackFilter {
        TrackFilter {
            cyl_start: Some(0),
            cyl_end: Some(79),
            head: None,
        }
    }
}

fn parse_amiga_sector<'a>(
    iterator: &mut impl Iterator<Item = &'a RawMfmWord>,
    expected_track_number: u32,
) -> anyhow::Result<CollectedSector> {
    let mut sector_header_odd = read_even_bits(iterator);
    if sector_header_odd == 0 {
        // filter out a potential sync word.
        // the real sector header odd is never 0
        sector_header_odd = read_even_bits(iterator);
    }

    let sector_header_even = read_even_bits(iterator);
    let sector_header = ((sector_header_odd) << 1) | (sector_header_even);

    // every sector header must start with 0xff
    ensure!(
        sector_header & 0xff00_0000 == 0xff00_0000,
        "Sector header not starting with 0xff {:x}",
        sector_header
    );

    let track = (sector_header >> 16) & 0xff;
    let sector = (sector_header >> 8) & 0xff;

    ensure!(
        expected_track_number == track,
        "Sector {} has not expected track {} != {}",
        sector,
        expected_track_number,
        track
    );

    let mut checksum: u32 = 0;
    checksum ^= sector_header_odd;
    checksum ^= sector_header_even;

    // discard sector label (odd)
    checksum ^= read_even_bits(iterator);
    checksum ^= read_even_bits(iterator);
    checksum ^= read_even_bits(iterator);
    checksum ^= read_even_bits(iterator);

    // discard sector label (even)
    checksum ^= read_even_bits(iterator);
    checksum ^= read_even_bits(iterator);
    checksum ^= read_even_bits(iterator);
    checksum ^= read_even_bits(iterator);

    // header checksum
    checksum ^= read_even_bits(iterator);
    checksum ^= read_even_bits(iterator);

    ensure!(checksum == 0);

    // start with data checksum
    checksum ^= read_even_bits(iterator);
    checksum ^= read_even_bits(iterator);

    let mut sector_data: Vec<u8> = Vec::with_capacity(512);

    // TODO is this really efficient code?
    // now get the odd data
    for _ in 0..WORDS_PER_SECTOR {
        let word = read_even_bits(iterator);
        checksum ^= word;
        sector_data.extend_from_slice(&(word << 1).to_be_bytes())
    }
    assert_eq!(sector_data.len(), 512);

    // now get the even data
    for target in sector_data.chunks_mut(4) {
        let word = read_even_bits(iterator);

        checksum ^= word;
        let target2: &mut [u8; 4] = target.try_into().unwrap();
        *target2 = (word | u32::from_be_bytes(*target2)).to_be_bytes();
    }

    ensure!(
        checksum == 0,
        "Checksum of data in sector {} {} is wrong",
        track,
        sector
    );

    Ok(CollectedSector {
        index: sector,
        payload: sector_data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_reader::image_adf::generate_track;
    use std::vec;
    use util::{bitstream::to_bit_stream, fluxpulse::FluxPulseGenerator};
    const BYTES_PER_SECTOR: usize = WORDS_PER_SECTOR * 4;
    use rand::{rngs::SmallRng, RngCore, SeedableRng};

    #[test]
    fn track_parse_test() {
        let mut rng = SmallRng::seed_from_u64(0x42);
        let mut buffer = vec![0; BYTES_PER_SECTOR * SECTORS_PER_AMIGA_DD_TRACK];
        rng.fill_bytes(&mut buffer);

        let mut sectors = buffer.chunks_exact(BYTES_PER_SECTOR);
        assert_eq!(sectors.len(), 11);

        let trackbuf = generate_track(30, 1, &mut sectors);
        let mut pulse_data = Vec::new();
        let mut pulse_generator = FluxPulseGenerator::new(|f| pulse_data.push(f.0 as u8), 168 >> 3);
        for i in trackbuf {
            to_bit_stream(i, |bit| pulse_generator.feed(bit));
        }
        // append some data to allow and ending pulse
        to_bit_stream(0x55, |bit| pulse_generator.feed(bit));
        pulse_generator.flush();

        let mut parser = AmigaTrackParser::new(Density::SingleDouble);
        parser.expect_track(30, 1);
        let result = parser.parse_raw_track(&pulse_data).unwrap();

        // Check parsed track is equal to data which was used to generate the track
        assert_eq!(buffer, result.payload);
        // just to be sure that we used pseudo random values
        assert_eq!(result.payload[100], 152);
        assert_eq!(result.payload[200], 126);
        assert_eq!(result.payload[300], 83);
    }
}
