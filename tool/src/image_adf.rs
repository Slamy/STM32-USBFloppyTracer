use util::bitstream::BitStreamCollector;
use util::mfm::MfmEncoder;
use util::mfm::MfmWord;
use util::{Bit, DensityMapEntry, PulseDuration};

use std::convert::TryInto;
use std::fs::{self, File};
use std::io::Read;
use std::slice::ChunksExact;

use crate::rawtrack::RawTrack;

// info from http://lclevy.free.fr/adflib/adf_info.html

const AMIGA_MFM_MASK: u32 = 0x55555555;
const SECTORS_PER_TRACK: u32 = 11;

const CYLINDERS: u32 = 80;
const HEADS: u32 = 2;
const BYTES_PER_SECTOR: u32 = 512;

fn generate_amiga_sector<T>(
    cylinder: u32,
    head: u32,
    sector: u32,
    sectordata: &[u8],
    encoder: &mut MfmEncoder<T>,
) where
    T: FnMut(Bit),
{
    // Preamble of 0xAAAA AAAA
    encoder.feed_encoded8(0);
    encoder.feed_encoded8(0);

    // 2x Sync Word 0x4489 4489
    encoder.feed(MfmWord::SyncWord);
    encoder.feed(MfmWord::SyncWord);

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
    let amiga_sectorHeader: u32 =
        0xff000000 | (cylinder << 17) | (head << 16) | (sector << 8) | (SECTORS_PER_TRACK - sector);

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
        ((amiga_sectorHeader >> 1) & AMIGA_MFM_MASK) ^ (amiga_sectorHeader & AMIGA_MFM_MASK),
    );

    assert!(sectordata.len() == 512);

    let mut checksum: u32 = 0;
    let longs = sectordata.chunks(4);
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
    let longs = sectordata.chunks(4);
    assert!(longs.len() == 128);
    for long in longs {
        encoder.feed_odd16_32(u32::from_be_bytes(long.try_into().unwrap()));
    }

    // then even data
    let longs = sectordata.chunks(4);
    assert!(longs.len() == 128);
    for long in longs {
        encoder.feed_even16_32(u32::from_be_bytes(long.try_into().unwrap()));
    }
}

fn generate_amiga_track(cylinder: u32, head: u32, sectors: &mut ChunksExact<u8>) -> Vec<u8> {
    let mut trackbuf: Vec<u8> = Vec::new();
    let mut collector = BitStreamCollector::new(|f| trackbuf.push(f));
    let mut encoder = MfmEncoder::new(|cell| collector.feed(cell));

    for sector in 0..SECTORS_PER_TRACK {
        let sectordata = sectors.next().unwrap();

        generate_amiga_sector(cylinder, head, sector, sectordata, &mut encoder);
    }

    // provide some fluxes to end the track properly.
    encoder.feed_even16_32(0);
    encoder.feed_even16_32(0);

    trackbuf
}

pub fn parse_adf_image(path: &str) -> Vec<RawTrack> {
    println!("Reading ADF from {} ...", path);

    let mut f = File::open(&path).expect("no file found");
    let metadata = fs::metadata(&path).expect("unable to read metadata");
    assert_eq!(
        metadata.len() as u32,
        BYTES_PER_SECTOR * HEADS * SECTORS_PER_TRACK * CYLINDERS
    );
    let mut buffer = vec![0; metadata.len() as usize];

    let bytes_read = f.read(&mut buffer).expect("buffer overflow");
    assert!(bytes_read == metadata.len() as usize);

    let mut sectors = buffer.chunks_exact(BYTES_PER_SECTOR as usize);

    let mut tracks: Vec<RawTrack> = Vec::new();

    for cylinder in 0..CYLINDERS {
        for head in 0..HEADS {
            let trackbuf = generate_amiga_track(cylinder, head, &mut sectors);

            let densitymap = vec![DensityMapEntry {
                number_of_cells: trackbuf.len() as usize,
                cell_size: PulseDuration(168),
            }];

            tracks.push(RawTrack::new(
                cylinder,
                head,
                trackbuf,
                densitymap,
                util::Encoding::MFM,
            ));
        }
    }

    tracks
}

#[cfg(test)]
mod tests {
    use std::convert::TryInto;

    use crate::image_adf::AMIGA_MFM_MASK;

    use super::{generate_amiga_track, BYTES_PER_SECTOR, SECTORS_PER_TRACK};

    fn check_aligned_amiga_mfm_track(buffer: &Vec<u8>) {
        let mut longs = buffer.chunks(4);

        for _ in 0..SECTORS_PER_TRACK {
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
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;

            // discard sector label (even)
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;

            // header checksum
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;

            println!("Header Checksum {:x}", checksum);
            assert_eq!(checksum, 0);

            // start with data checksum
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
            checksum ^=
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;

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

    #[test]
    fn amiga_track_check_test() {
        let buffer = vec![0x12; (BYTES_PER_SECTOR * SECTORS_PER_TRACK) as usize];
        let mut sectors = buffer.chunks_exact(BYTES_PER_SECTOR as usize);

        let trackbuf = generate_amiga_track(30, 1, &mut sectors);
        check_aligned_amiga_mfm_track(&trackbuf);
    }
}
