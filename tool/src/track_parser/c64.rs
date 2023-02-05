use anyhow::ensure;
use util::{
    c64_geometry::{get_track_settings, TrackConfiguration},
    duration_of_rotation_as_stm_tim_raw,
    fluxpulse::FluxPulseToCells,
    gcr::{GcrDecoder, GcrDecoderResult},
    Density, PulseDuration, DRIVE_5_25_RPM,
};

use crate::{rawtrack::TrackFilter, track_parser::concatenate_sectors};

use super::{CollectedSector, TrackParser, TrackPayload};

pub struct C64TrackParser {
    collected_sectors: Option<Vec<CollectedSector>>,
    track_config: Option<TrackConfiguration>,
    expected_track_number: Option<u32>,
}

impl C64TrackParser {
    pub fn new() -> Self {
        C64TrackParser {
            collected_sectors: None,
            track_config: None,
            expected_track_number: None,
        }
    }
}

impl Default for C64TrackParser {
    fn default() -> Self {
        Self::new()
    }
}

impl TrackParser for C64TrackParser {
    fn default_file_extension(&self) -> &str {
        "d64"
    }

    fn format_name(&self) -> &str {
        "C64 1541"
    }

    fn duration_to_record(&self) -> usize {
        duration_of_rotation_as_stm_tim_raw(DRIVE_5_25_RPM) * 110 / 100
    }

    fn track_density(&self) -> Density {
        Density::SingleDouble
    }

    fn default_trackfilter(&self) -> crate::rawtrack::TrackFilter {
        TrackFilter {
            cyl_start: Some(0),
            cyl_end: Some(68),
            head: Some(0),
        }
    }

    fn parse_raw_track(&mut self, track: &[u8]) -> anyhow::Result<TrackPayload> {
        let track_config = self.track_config.as_ref().unwrap();

        let mut gcr_results = Vec::new();
        let mut decoder = GcrDecoder::new(|f| gcr_results.push(f));
        let mut pulseparser =
            FluxPulseToCells::new(|val| decoder.feed(val), track_config.cellsize as i32);

        track
            .iter()
            .for_each(|f| pulseparser.feed(PulseDuration((*f as i32) << 3)));

        let mut iterator = gcr_results.iter();

        let mut awaiting_data_block = 0;
        let mut sector_header = Vec::new();

        // Search for Syncs until the end.
        while let Some(searchword) = iterator.next() {
            awaiting_data_block -= 1;

            if matches!(searchword, GcrDecoderResult::Sync) {
                let block_type = iterator.next();

                match block_type {
                    Some(GcrDecoderResult::Byte(0x08)) => {
                        //Header Block

                        sector_header.clear();

                        for _ in 0..5 {
                            if let Some(GcrDecoderResult::Byte(val)) = iterator.next() {
                                sector_header.push(*val);
                            } else {
                                break;
                            }
                        }

                        let checksum = sector_header
                            .iter()
                            .cloned()
                            .reduce(|accu, input| accu ^ input)
                            .unwrap();

                        if sector_header.len() == 5 && checksum == 0 {
                            // Did we get this sector yet?

                            let collected_sectors = self.collected_sectors.as_mut().unwrap();

                            if !collected_sectors
                                .iter()
                                .any(|f| f.index == sector_header[1] as u32)
                            {
                                // Activate DAM reading for the next 40 data bytes
                                awaiting_data_block = 20;
                            }
                            ensure!(sector_header[2] as u32 == self.expected_track_number.unwrap());
                        } else {
                            println!("Checksum of sector {} header was wrong", sector_header[1])
                        }
                    }

                    Some(GcrDecoderResult::Byte(0x07)) if awaiting_data_block > 0 => {
                        // Data Block
                        let sector_size = 256;
                        let mut sector_data = Vec::with_capacity(sector_size + 1);

                        for _ in 0..sector_size + 1 {
                            if let Some(GcrDecoderResult::Byte(val)) = iterator.next() {
                                sector_data.push(*val);
                            } else {
                                break;
                            }
                        }

                        let checksum = sector_data
                            .iter()
                            .cloned()
                            .reduce(|accu, input| accu ^ input)
                            .unwrap();

                        if checksum == 0 {
                            let collected_sectors = self.collected_sectors.as_mut().unwrap();

                            sector_data.resize(sector_size, 0); // remove checksum at the end
                            collected_sectors.push(CollectedSector {
                                index: sector_header[1] as u32,
                                payload: sector_data,
                            });

                            if collected_sectors.len() == track_config.sectors as usize {
                                // Exit it after we got all expected sectors.
                            }
                        } else {
                            println!("Checksum of sector {} data was wrong", sector_header[1])
                        }
                    }
                    _ => {}
                }
            }
        }

        ensure!(self.collected_sectors.as_ref().unwrap().len() == track_config.sectors as usize);
        let collected_sectors = self.collected_sectors.take().unwrap();

        Ok(concatenate_sectors(
            collected_sectors,
            (self.expected_track_number.unwrap() - 1) << 1,
            0,
        ))
    }

    fn expect_track(&mut self, cylinder: u32, head: u32) {
        assert_eq!(head, 0, "C64 disks have no second side!");
        let expected_track_number = (cylinder >> 1) + 1;
        let track_config = get_track_settings(expected_track_number as usize);

        self.track_config = Some(track_config);
        self.expected_track_number = Some(expected_track_number);
        self.collected_sectors = Some(Vec::new());
    }

    fn step_size(&self) -> usize {
        2
    }
}