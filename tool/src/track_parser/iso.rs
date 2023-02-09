use anyhow::ensure;
use util::{
    duration_of_rotation_as_stm_tim_raw,
    fluxpulse::FluxPulseToCells,
    mfm::{MfmDecoder, MfmWord},
    Density, DiskType, PulseDuration, DRIVE_3_5_RPM, DRIVE_5_25_RPM, DRIVE_SLOWEST_RPM,
};

use crate::{rawtrack::TrackFilter, track_parser::concatenate_sectors};

use super::{CollectedSector, TrackParser, TrackPayload};

pub struct IsoTrackParser {
    collected_sectors: Option<Vec<CollectedSector>>,
    expected_sectors_per_track: Option<usize>,
    expected_cylinder: Option<u32>,
    expected_head: Option<u32>,
    density: Density,
    assumed_disk_type: Option<DiskType>,
}

impl IsoTrackParser {
    pub fn new(expected_sectors_per_track: Option<usize>, density: Density) -> Self {
        IsoTrackParser {
            collected_sectors: None,
            expected_sectors_per_track,
            expected_cylinder: None,
            expected_head: None,
            density,
            assumed_disk_type: None,
        }
    }
}

impl TrackParser for IsoTrackParser {
    fn default_file_extension(&self) -> &str {
        match self.density {
            Density::High => "img",
            Density::SingleDouble => "st",
        }
    }

    fn format_name(&self) -> &str {
        match self.density {
            Density::High => "High Density ISO - could be MS-DOS",
            Density::SingleDouble => "Double Density ISO - could be Atari ST",
        }
    }

    fn duration_to_record(&self) -> usize {
        let rpm = match self.assumed_disk_type {
            Some(DiskType::Inch3_5) => DRIVE_3_5_RPM,
            Some(DiskType::Inch5_25) => DRIVE_5_25_RPM,
            None => DRIVE_SLOWEST_RPM,
        };

        let percent = match self.density {
            Density::High => 106,
            Density::SingleDouble => 110,
        };
        duration_of_rotation_as_stm_tim_raw(rpm) * percent / 100
    }

    fn track_density(&self) -> Density {
        self.density
    }

    fn default_trackfilter(&self) -> crate::rawtrack::TrackFilter {
        TrackFilter {
            cyl_start: Some(0),
            cyl_end: Some(79),
            head: None,
        }
    }
    fn parse_raw_track(&mut self, track: &[u8]) -> anyhow::Result<TrackPayload> {
        //println!("{:?}", track);

        let mut mfm_words: Vec<MfmWord> = Vec::new();
        let mut mfmd = MfmDecoder::new(|f| mfm_words.push(f));

        let cellsize = match self.density {
            Density::High => 84,
            Density::SingleDouble => 168,
        };

        let mut pulseparser = FluxPulseToCells::new(|val| mfmd.feed(val), cellsize);

        // TODO magic number
        track
            .iter()
            .for_each(|f| pulseparser.feed(PulseDuration((*f as i32) << 3)));

        let mut iterator = mfm_words.iter();

        let mut awaiting_dam = 0;
        let mut sector_header = Vec::new();
        let mut number_of_duplicate_sector_headers_found_in_stream = 0;

        // Search for Syncs until the end.
        while let Some(searchword) = iterator.next() {
            awaiting_dam -= 1;

            if matches!(searchword, MfmWord::SyncWord) {
                let address_mark_type = iterator.next();

                //println!("{} {:x?}", awaiting_dam, address_mark_type);

                match address_mark_type {
                    Some(MfmWord::Enc(0xfe)) => {
                        sector_header.clear();

                        for _ in 0..6 {
                            if let Some(MfmWord::Enc(val)) = iterator.next() {
                                sector_header.push(*val);
                            }
                        }

                        let mut crc = crc16::State::<crc16::CCITT_FALSE>::new();
                        crc.update(&[0xa1, 0xa1, 0xa1, 0xfe]);
                        crc.update(&sector_header);
                        let crc16 = crc.get();
                        if crc16 == 0 {
                            //println!("Got sector header {:?}", sector_header);
                            // Did we get this sector yet?
                            let collected_sectors = self.collected_sectors.as_mut().unwrap();

                            if !collected_sectors
                                .iter()
                                .any(|f| f.index == sector_header[2] as u32)
                            {
                                // Activate DAM reading for the next 40 data bytes
                                awaiting_dam = 40;
                            } else {
                                number_of_duplicate_sector_headers_found_in_stream += 1;
                            }
                            ensure!(sector_header[0] as u32 == self.expected_cylinder.unwrap());
                            ensure!(sector_header[1] as u32 == self.expected_head.unwrap());
                        }
                    }
                    Some(MfmWord::Enc(0xfb)) if awaiting_dam > 0 => {
                        let sector_size = 128 << sector_header[3];
                        let mut sector_data = Vec::with_capacity(sector_size + 2);

                        for _ in 0..sector_size + 2 {
                            if let Some(MfmWord::Enc(val)) = iterator.next() {
                                sector_data.push(*val);
                            } else {
                                break;
                            }
                        }

                        let mut crc = crc16::State::<crc16::CCITT_FALSE>::new();
                        crc.update(&[0xa1, 0xa1, 0xa1, 0xfb]);
                        crc.update(&sector_data);
                        let crc16 = crc.get();
                        if crc16 == 0 {
                            let collected_sectors = self.collected_sectors.as_mut().unwrap();

                            sector_data.resize(sector_size, 0); // remove CRC at the end
                            collected_sectors.push(CollectedSector {
                                index: sector_header[2] as u32,
                                payload: sector_data,
                            });

                            if let Some(expected_sectors_per_track) = self.expected_sectors_per_track &&
                                expected_sectors_per_track == collected_sectors.len()
                            {
                                // Exit it after we got all expected sectors.
                                break;
                            }
                        } else {
                            println!("CRC Error Sector {}", sector_header[2]);
                        }
                    }
                    _ => {}
                }
            }
        }

        // we need to at least have one sector. if not, this read was not successful at all
        ensure!(self.collected_sectors.as_ref().unwrap().is_empty() == false);

        self.assumed_disk_type.get_or_insert_with(|| {
            println!(
                "Number of duplicate sectors in stream: {}",
                number_of_duplicate_sector_headers_found_in_stream
            );
            if number_of_duplicate_sector_headers_found_in_stream > 5 {
                println!("Assume 5.25 inch drive.");
                DiskType::Inch5_25
            } else {
                println!("Assume 3.5 inch drive.");
                DiskType::Inch3_5
            }
        });

        assert!(self.assumed_disk_type.is_some());

        // The number of sectors must match our expectations in case they exist
        if let Some(expected_sectors_per_track) = self.expected_sectors_per_track {
            ensure!(self.collected_sectors.as_ref().unwrap().len() == expected_sectors_per_track);
        } else {
            // But for the next tracks, I really want them to match to be more safe here.
            // Flukes in reading the first track will cause a fail in the next as the sector
            // numbers won't match on the next.
            let collected_sector_number = self.collected_sectors.as_ref().unwrap().len();

            println!(
                "Assume {} sectors per track from now on...",
                collected_sector_number
            );
            self.expected_sectors_per_track = Some(collected_sector_number);
        }

        let collected_sectors = self.collected_sectors.take().unwrap();

        Ok(concatenate_sectors(
            collected_sectors,
            self.expected_cylinder.unwrap(),
            self.expected_head.unwrap(),
        ))
    }

    fn expect_track(&mut self, cylinder: u32, head: u32) {
        self.expected_cylinder = Some(cylinder);
        self.expected_head = Some(head);
        self.collected_sectors = Some(Vec::new());
    }

    fn step_size(&self) -> usize {
        1
    }
}
