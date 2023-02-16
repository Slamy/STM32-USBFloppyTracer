use crate::rawtrack::{RawImage, RawTrack};
use std::cell::RefCell;
use std::fs::{self, File};
use std::io::Read;
use util::bitstream::{to_bit_stream, BitStreamCollector};
use util::c64_geometry::get_track_settings;
use util::gcr::to_gcr_stream;
use util::{DensityMapEntry, PulseDuration};

// http://www.baltissen.org/newhtm/1541c.htm

#[must_use]
pub fn parse_d64_image(path: &str) -> RawImage {
    println!("Reading D64 from {path} ...");

    let mut file = File::open(path).expect("no file found");
    let metadata = fs::metadata(path).expect("unable to read metadata");
    assert_eq!(metadata.len() as u32, 174_848, "D64 image has wrong size");

    let cylinders: u8 = 35;
    let bytes_per_sector = 256;

    // Nothing specific as disk id. Just something random.
    let id1 = 0x39_u8;
    let id2 = 0x30_u8;

    let mut tracks: Vec<RawTrack> = Vec::new();

    for src_cylinder in 0..cylinders {
        let tracknum = src_cylinder + 1;
        let settings = get_track_settings(tracknum as usize);
        let mut trackbuf: Vec<u8> = Vec::new();
        let c64header_track = src_cylinder + 1;

        for sector in 0..settings.sectors {
            let mut sector_buffer = vec![0; bytes_per_sector];
            let bytes_read = file.read(&mut sector_buffer).expect("buffer overflow");
            assert!(bytes_read == bytes_per_sector);

            let collector = RefCell::new(BitStreamCollector::new(|byte| trackbuf.push(byte)));
            let feed_raw = |word| to_bit_stream(word, |cell| collector.borrow_mut().feed(cell));
            let feed_gcr = |word| to_gcr_stream(word, |cell| collector.borrow_mut().feed(cell));

            // Header
            feed_raw(0xff);
            feed_raw(0xff);
            feed_raw(0xff);
            feed_raw(0xff);
            feed_raw(0xff);

            let checksum: u8 = sector ^ c64header_track ^ id1 ^ id2;
            feed_gcr(0x08);
            feed_gcr(checksum);
            feed_gcr(sector);
            feed_gcr(c64header_track);
            feed_gcr(id2);
            feed_gcr(id1);
            feed_gcr(0x0f);
            feed_gcr(0x0f);

            //Gap #3
            feed_raw(0x55);
            feed_raw(0x55);
            feed_raw(0x55);
            feed_raw(0x55);
            feed_raw(0x55);

            //Data
            feed_raw(0xff);
            feed_raw(0xff);
            feed_raw(0xff);
            feed_raw(0xff);
            feed_raw(0xff);

            let mut checksum = 0;
            feed_gcr(0x07);

            for byte in sector_buffer {
                feed_gcr(byte);
                checksum ^= byte;
            }
            feed_gcr(checksum);
            feed_gcr(0x00);
            feed_gcr(0x00);

            for _ in 0..settings.gap_size {
                feed_raw(0x55);
            }
        }

        let densitymap = vec![DensityMapEntry {
            number_of_cellbytes: trackbuf.len(),
            cell_size: PulseDuration(settings.cellsize as i32),
        }];

        tracks.push(RawTrack::new(
            u32::from(src_cylinder) * 2,
            0,
            trackbuf,
            densitymap,
            util::Encoding::GCR,
        ));
    }

    RawImage {
        tracks,
        disk_type: util::DiskType::Inch5_25,
        density: util::Density::SingleDouble,
    }
}
