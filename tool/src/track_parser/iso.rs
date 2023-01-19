use anyhow::ensure;
use util::{
    fluxpulse::FluxPulseToCells,
    mfm::{MfmDecoder, MfmWord},
    PulseDuration,
};

use crate::track_parser::concatenate_sectors;

use super::{CollectedSector, TrackParser, TrackPayload};

pub struct IsoTrackParser {
    collected_sectors: Option<Vec<CollectedSector>>,
    expected_sectors_per_track: usize,
    expected_cylinder: Option<u32>,
    expected_head: Option<u32>,
}

impl IsoTrackParser {
    pub fn new(expected_sectors_per_track: usize) -> Self {
        IsoTrackParser {
            collected_sectors: None,
            expected_sectors_per_track,
            expected_cylinder: None,
            expected_head: None,
        }
    }
}

impl TrackParser for IsoTrackParser {
    fn parse_raw_track(&mut self, track: &[u8]) -> anyhow::Result<TrackPayload> {
        //println!("{:?}", track);

        let mut mfm_words: Vec<MfmWord> = Vec::new();
        let mut mfmd = MfmDecoder::new(|f| mfm_words.push(f));
        // TODO cell duration magic number
        let mut pulseparser = FluxPulseToCells::new(|val| mfmd.feed(val), 84);

        // TODO magic number
        track
            .iter()
            .for_each(|f| pulseparser.feed(PulseDuration((*f as i32) << 3)));

        let mut iterator = mfm_words.iter();

        let mut awaiting_dam = 0;
        let mut sector_header = Vec::new();

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
                        crc.update(&vec![0xa1, 0xa1, 0xa1, 0xfe]);
                        crc.update(&sector_header);
                        let crc16 = crc.get();
                        if crc16 == 0 {
                            //println!("Got sector header {:?}", sector_header);

                            // Did we get this sector yet?

                            let collected_sectors = self.collected_sectors.as_mut().unwrap();

                            if collected_sectors
                                .iter()
                                .find(|f| f.index == sector_header[2] as u32)
                                .is_none()
                            {
                                // Activate DAM reading for the next 40 data bytes
                                awaiting_dam = 40;
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
                        crc.update(&vec![0xa1, 0xa1, 0xa1, 0xfb]);
                        crc.update(&sector_data);
                        let crc16 = crc.get();
                        if crc16 == 0 {
                            let collected_sectors = self.collected_sectors.as_mut().unwrap();

                            sector_data.resize(sector_size, 0); // remove CRC at the end
                            collected_sectors.push(CollectedSector {
                                index: sector_header[2] as u32,
                                payload: sector_data,
                            });

                            if collected_sectors.len() == self.expected_sectors_per_track {
                                // Exit it after we got all expected sectors.
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        ensure!(self.collected_sectors.as_ref().unwrap().len() == self.expected_sectors_per_track);
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
