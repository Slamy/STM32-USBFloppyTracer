use crate::rawtrack::{RawImage, RawTrack};
use std::cell::RefCell;
use std::fs::{self, File};
use std::io::Read;
use std::slice::ChunksExact;
use util::bitstream::{to_bit_stream, BitStreamCollector};
use util::c64_geometry::{get_track_settings, TrackConfiguration};
use util::gcr::to_gcr_stream;
use util::{DensityMapEntry, PulseDuration};

// http://www.baltissen.org/newhtm/1541c.htm

const CYLINDERS: u8 = 35;
const SECTORS_TOTAL: usize = 683;
const BYTES_PER_SECTOR: usize = 256;

// Nothing specific as disk id. Just something random.
const ID1: u8 = 0x39_u8;
const ID2: u8 = 0x30_u8;

pub fn generate_track(
    tracknum: u8,
    sectors: &mut ChunksExact<u8>,
) -> (Vec<u8>, TrackConfiguration) {
    let settings = get_track_settings(tracknum as usize);
    let mut trackbuf: Vec<u8> = Vec::new();
    assert!(
        sectors.len() >= settings.sectors as usize,
        "Not enough sectors for this track"
    );

    for sector in 0..settings.sectors {
        let sector_buffer = sectors.next().expect("Not enough sectors for this track");
        assert!(sector_buffer.len() == BYTES_PER_SECTOR);

        let collector = RefCell::new(BitStreamCollector::new(|byte| trackbuf.push(byte)));
        let feed_raw = |word| to_bit_stream(word, |cell| collector.borrow_mut().feed(cell));
        let feed_gcr = |word| to_gcr_stream(word, |cell| collector.borrow_mut().feed(cell));

        // Header
        feed_raw(0xff);
        feed_raw(0xff);
        feed_raw(0xff);
        feed_raw(0xff);
        feed_raw(0xff);

        let checksum: u8 = sector ^ tracknum ^ ID1 ^ ID2;
        feed_gcr(0x08);
        feed_gcr(checksum);
        feed_gcr(sector);
        feed_gcr(tracknum);
        feed_gcr(ID2);
        feed_gcr(ID1);
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
            feed_gcr(*byte);
            checksum ^= byte;
        }
        feed_gcr(checksum);
        feed_gcr(0x00);
        feed_gcr(0x00);

        for _ in 0..settings.gap_size {
            feed_raw(0x55);
        }
    }
    (trackbuf, settings)
}

#[must_use]
pub fn parse_d64_image(path: &str) -> RawImage {
    println!("Reading D64 from {path} ...");

    let mut file = File::open(path).expect("no file found");
    let metadata = fs::metadata(path).expect("unable to read metadata");

    let mut whole_file_buffer: Vec<u8> = vec![0; metadata.len() as usize];
    let bytes_read = file.read(whole_file_buffer.as_mut()).unwrap();
    assert_eq!(bytes_read, metadata.len() as usize);

    assert_eq!(metadata.len() as u32, 174_848, "D64 image has wrong size");

    let mut tracks: Vec<RawTrack> = Vec::new();
    let mut sectors = whole_file_buffer.chunks_exact(BYTES_PER_SECTOR);
    assert_eq!(sectors.len(), SECTORS_TOTAL);

    for src_cylinder in 0..CYLINDERS {
        let tracknum = src_cylinder + 1;

        let (trackbuf, settings) = generate_track(tracknum, &mut sectors);

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
