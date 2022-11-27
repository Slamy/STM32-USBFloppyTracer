use util::bitstream::BitStreamCollector;
use util::{DensityMapEntry, MfmEncoder2, PulseDuration};

use std::convert::TryInto;
use std::fs::{self, File};
use std::io::Read;

use crate::rawtrack::RawTrack;
// info from http://lclevy.free.fr/adflib/adf_info.html

const AMIGA_MFM_MASK: u32 = 0x55555555;

pub fn check_aligned_amiga_mfm_track(buffer: &Vec<u8>) {
    let mut longs = buffer.chunks(4);
    const AMIGA_MFM_MASK: u32 = 0x55555555;

    for _ in 0..11 {
        loop {
            let longbuf = longs.next().unwrap();
            let long = u32::from_be_bytes(longbuf.try_into().unwrap());

            if long == 0x44894489 {
                println!("Detected sync!");
                break;
            }
        }

        let sector_header_odd =
            u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
        let sector_header_even =
            u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;

        let sector_header = ((sector_header_odd) << 1) | (sector_header_even);

        println!("{:x}", sector_header);
        assert_eq!(sector_header & 0xFF000000, 0xff000000);
        let track = (sector_header >> 16) & 0xff;
        let sector = (sector_header >> 8) & 0xff;
        let remaining_sectors = sector_header & 0xff;
        println!("Track {} Sector {}", track, sector);
        assert_eq!(sector, 11 - remaining_sectors);

        let mut checksum: u32 = 0;
        checksum ^= sector_header_odd;
        checksum ^= sector_header_even;

        // discard sector label (odd)
        checksum ^= u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
        checksum ^= u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
        checksum ^= u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
        checksum ^= u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;

        // discard sector label (even)
        checksum ^= u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
        checksum ^= u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
        checksum ^= u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
        checksum ^= u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;

        // header checksum
        checksum ^= u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
        checksum ^= u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;

        println!("Header Checksum {:x}", checksum);
        assert_eq!(checksum, 0);

        // start with data checksum
        checksum ^= u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
        checksum ^= u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;

        // now get the sector data
        for _ in 0..128 {
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
        }

        println!("Data Checksum {:x}", checksum);
        assert_eq!(checksum, 0);
    }
}

pub fn parse_adf_image(path: &str) -> Vec<RawTrack> {
    println!("Reading ADF from {} ...", path);

    let cylinders: u32 = 80;
    let heads = 2;
    let sectors_per_track = 11;
    let bytes_per_sector = 512;

    let mut f = File::open(&path).expect("no file found");
    let metadata = fs::metadata(&path).expect("unable to read metadata");
    assert_eq!(
        metadata.len() as u32,
        bytes_per_sector * heads * sectors_per_track * cylinders
    );
    let mut buffer = vec![0; metadata.len() as usize];

    let bytes_read = f.read(&mut buffer).expect("buffer overflow");
    assert!(bytes_read == metadata.len() as usize);

    let mut sectors = buffer.chunks_exact(bytes_per_sector as usize);

    let mut tracks: Vec<RawTrack> = Vec::new();

    for cylinder in 0..cylinders {
        for head in 0..heads {
            let mut trackbuf: Vec<u8> = Vec::new();
            let mut collector = BitStreamCollector::new(|f| trackbuf.push(f));
            let mut encoder = MfmEncoder2::new(|cell| collector.feed(cell));

            for sector in 0..sectors_per_track {
                // Preamble of 0xAAAA AAAA
                encoder.feed_encoded8(0);
                encoder.feed_encoded8(0);

                // 2x Sync Word 0x4489 4489
                encoder.feed(util::MfmResult2::SyncWord);
                encoder.feed(util::MfmResult2::SyncWord);

                /*
                 * decoded long is : 0xFF TT SS SG
                 * 0xFF = Amiga v1.0 format
                 * TT = track number ( 3 means cylinder 1, head 1)
                 * SS = sector number ( 0 upto 10/21 )
                 *    sectors are not ordered !!!
                 * SG = sectors until end of writing (including
                 *   current one)
                 */
                assert!(head < 2);
                let amiga_sectorHeader: u32 = 0xff000000
                    | (cylinder << 17)
                    | (head << 16)
                    | (sector << 8)
                    | (sectors_per_track - sector);

                encoder.feed_odd16_32(amiga_sectorHeader);
                encoder.feed_even16_32(amiga_sectorHeader);

                // Sector Label Area : OS recovery info, reserved for future use
                encoder.feed_odd16_32(0); // 4 odd
                encoder.feed_odd16_32(0);
                encoder.feed_odd16_32(0);
                encoder.feed_odd16_32(0);

                encoder.feed_even16_32(0); // 4 even
                encoder.feed_even16_32(0);
                encoder.feed_even16_32(0);
                encoder.feed_even16_32(0);

                // header checksum
                encoder.feed_odd16_32(0);
                encoder.feed_even16_32(
                    ((amiga_sectorHeader >> 1) & AMIGA_MFM_MASK)
                        ^ (amiga_sectorHeader & AMIGA_MFM_MASK),
                );

                let sector = sectors.next().unwrap();
                assert!(sector.len() == 512);

                let mut checksum: u32 = 0;
                let longs = sector.chunks(4);
                assert!(longs.len() == 128);

                for long in longs {
                    let word: u32 = u32::from_be_bytes(long.try_into().unwrap());

                    checksum ^= word & AMIGA_MFM_MASK;
                    checksum ^= (word >> 1) & AMIGA_MFM_MASK;
                }

                // data checksum
                encoder.feed_odd16_32(0);
                encoder.feed_even16_32(checksum);

                // first odd data
                let longs = sector.chunks(4);
                assert!(longs.len() == 128);
                for long in longs {
                    encoder.feed_odd16_32(u32::from_be_bytes(long.try_into().unwrap()));
                }

                // then even data
                let longs = sector.chunks(4);
                assert!(longs.len() == 128);
                for long in longs {
                    encoder.feed_even16_32(u32::from_be_bytes(long.try_into().unwrap()));
                }
            }

            // provide some fluxes to end the track properly.
            encoder.feed_even16_32(0);
            encoder.feed_even16_32(0);

            // check_aligned_amiga_mfm_track(&trackbuf);

            let densitymap = vec![DensityMapEntry {
                number_of_cells: trackbuf.len() as usize,
                cell_size: PulseDuration(168),
            }];

            tracks.push(RawTrack::new(cylinder, head, trackbuf, densitymap));
        }
    }

    tracks
}
