use crate::rawtrack::RawImage;
use crate::rawtrack::RawTrack;
use anyhow::ensure;
use anyhow::Context;
use std::convert::TryInto;
use std::fs::{self, File};
use std::io::Read;
use std::slice::ChunksExact;
use util::bitstream::BitStreamCollector;
use util::mfm::MfmEncoder;
use util::mfm::MfmWord;
use util::{Bit, DensityMapEntry, PulseDuration};

// info from http://lclevy.free.fr/adflib/adf_info.html

const AMIGA_MFM_MASK: u32 = 0x5555_5555;
const SECTORS_PER_TRACK: u32 = 11;

const CYLINDERS: u32 = 80;
const HEADS: u32 = 2;
const BYTES_PER_SECTOR: u32 = 512;

fn generate_sector<T>(
    cylinder: u32,
    head: u32,
    sector: u32,
    sectordata: &[u8],
    encoder: &mut MfmEncoder<T>,
) -> anyhow::Result<()>
where
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
    ensure!(head < 2);
    let amiga_sectorHeader: u32 = 0xff00_0000
        | (cylinder << 17)
        | (head << 16)
        | (sector << 8)
        | (SECTORS_PER_TRACK - sector);

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

    ensure!(sectordata.len() == 512);

    let mut checksum: u32 = 0;
    let longs = sectordata.chunks(4);
    ensure!(longs.len() == 128);

    for long in longs {
        let word: u32 = u32::from_be_bytes(long.try_into()?);

        checksum ^= word & AMIGA_MFM_MASK;
        checksum ^= (word >> 1) & AMIGA_MFM_MASK;
    }

    // data checksum
    encoder.feed_odd16_32(0);
    encoder.feed_even16_32(checksum);

    // first odd data
    let longs = sectordata.chunks(4);
    ensure!(longs.len() == 128);
    for long in longs {
        encoder.feed_odd16_32(u32::from_be_bytes(long.try_into()?));
    }

    // then even data
    let longs = sectordata.chunks(4);
    ensure!(longs.len() == 128);
    for long in longs {
        encoder.feed_even16_32(u32::from_be_bytes(long.try_into()?));
    }

    Ok(())
}

pub fn generate_track(
    cylinder: u32,
    head: u32,
    sectors: &mut ChunksExact<u8>,
) -> anyhow::Result<Vec<u8>> {
    let mut trackbuf: Vec<u8> = Vec::new();
    let mut collector = BitStreamCollector::new(|f| trackbuf.push(f));
    let mut encoder = MfmEncoder::new(|cell| collector.feed(cell));

    for sector in 0..SECTORS_PER_TRACK {
        let sectordata = sectors.next().context(program_flow_error!())?;

        generate_sector(cylinder, head, sector, sectordata, &mut encoder)?;
    }

    Ok(trackbuf)
}

pub fn parse_adf_image(path: &str) -> anyhow::Result<RawImage> {
    println!("Reading ADF from {path} ...");

    let mut f = File::open(path).context("no file found")?;
    let metadata = fs::metadata(path).context("unable to read metadata")?;
    ensure!(metadata.len() as u32 == BYTES_PER_SECTOR * HEADS * SECTORS_PER_TRACK * CYLINDERS);
    let mut buffer = vec![0; metadata.len() as usize];

    let bytes_read = f.read(&mut buffer).context("buffer overflow")?;
    ensure!(bytes_read == metadata.len() as usize);

    let mut sectors = buffer.chunks_exact(BYTES_PER_SECTOR as usize);

    let mut tracks: Vec<RawTrack> = Vec::new();

    for cylinder in 0..CYLINDERS {
        for head in 0..HEADS {
            let trackbuf = generate_track(cylinder, head, &mut sectors)?;

            let densitymap = vec![DensityMapEntry {
                number_of_cellbytes: trackbuf.len(),
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

    Ok(RawImage {
        tracks,
        density: util::Density::SingleDouble,
        disk_type: util::DiskType::Inch3_5,
    })
}

#[cfg(test)]
mod tests {
    use std::convert::TryInto;

    use super::*;

    fn check_aligned_amiga_mfm_track(buffer: &[u8]) {
        let mut longs = buffer.chunks(4);

        for _ in 0..SECTORS_PER_TRACK {
            loop {
                let longbuf = longs.next().unwrap();
                let long = u32::from_be_bytes(longbuf.try_into().unwrap());

                if long == 0x4489_4489 {
                    println!("Detected sync!");
                    break;
                }
            }

            let sector_header_odd =
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;
            let sector_header_even =
                u32::from_be_bytes(longs.next().unwrap().try_into().unwrap()) & AMIGA_MFM_MASK;

            let sector_header = ((sector_header_odd) << 1) | (sector_header_even);

            println!("{sector_header:x}");
            assert_eq!(sector_header & 0xFF00_0000, 0xff00_0000);
            let track = (sector_header >> 16) & 0xff;
            let sector = (sector_header >> 8) & 0xff;
            let remaining_sectors = sector_header & 0xff;
            println!("Track {track} Sector {sector}");
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

            println!("Header Checksum {checksum:x}");
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

            println!("Data Checksum {checksum:x}");
            assert_eq!(checksum, 0);
        }
    }

    #[test]
    fn track_check_test() {
        let buffer = vec![0x12; (BYTES_PER_SECTOR * SECTORS_PER_TRACK) as usize];
        let mut sectors = buffer.chunks_exact(BYTES_PER_SECTOR as usize);

        let trackbuf = generate_track(30, 1, &mut sectors).unwrap();
        check_aligned_amiga_mfm_track(&trackbuf);
    }
}
