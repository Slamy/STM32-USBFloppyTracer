use util::bitstream::{to_bit_stream, BitStreamCollector};
use util::{Bit, Cylinder, DensityMapEntry, PulseDuration};

use std::cell::RefCell;

use std::fs::{self, File};
use std::io::Read;

use crate::rawtrack::RawTrack;

// http://www.baltissen.org/newhtm/1541c.htm

const GCR_ENCODE_TABLE: [u8; 16] = [
    0b01010, //0000
    0b01011, //0001
    0b10010, //0010
    0b10011, //0011
    0b01110, //0100
    0b01111, //0101
    0b10110, //0110
    0b10111, //0111
    0b01001, //1000
    0b11001, //1001
    0b11010, //1010
    0b11011, //1011
    0b01101, //1100
    0b11101, //1101
    0b11110, //1110
    0b10101, //1111
];

pub fn to_gcr_stream<T>(byte: u8, mut sink: T)
where
    T: FnMut(Bit),
{
    let upper_nibble = byte >> 4;
    let lower_nibble = byte & 0xf;

    let mut gcr_word = (GCR_ENCODE_TABLE[upper_nibble as usize] as u16) << 5
        | GCR_ENCODE_TABLE[lower_nibble as usize] as u16;

    for _ in 0..10 {
        sink(Bit((gcr_word & (1 << 9)) != 0));
        gcr_word <<= 1;
    }
}

struct TrackConfiguration {
    cellsize: usize,
    sectors: u8,
    gap_size: usize,
}

fn get_track_settings(cyl: Cylinder) -> TrackConfiguration {
    if cyl.0 <= 16 {
        TrackConfiguration {
            cellsize: 227,
            sectors: 21,
            gap_size: 8,
        }
    } else if cyl.0 <= 23 {
        TrackConfiguration {
            cellsize: 245,
            sectors: 19,
            gap_size: 17,
        }
    } else if cyl.0 <= 29 {
        TrackConfiguration {
            cellsize: 262,
            sectors: 18,
            gap_size: 12,
        }
    } else {
        TrackConfiguration {
            cellsize: 280,
            sectors: 17,
            gap_size: 9,
        }
    }
}

pub fn parse_d64_image(path: &str) -> Vec<RawTrack> {
    println!("Reading D64 from {} ...", path);

    let mut f = File::open(&path).expect("no file found");
    let metadata = fs::metadata(&path).expect("unable to read metadata");
    assert_eq!(metadata.len() as u32, 174848, "D64 image has wrong size");
    let _buffer = vec![0; metadata.len() as usize];

    let cylinders: u8 = 35;
    let bytes_per_sector = 256;

    let id1 = 0x39_u8;
    let id2 = 0x30_u8;

    let mut tracks: Vec<RawTrack> = Vec::new();

    for src_cylinder in 0..cylinders {
        let settings = get_track_settings(Cylinder(src_cylinder));
        let mut trackbuf: Vec<u8> = Vec::new();
        let c64header_track = src_cylinder + 1;

        for sector in 0..settings.sectors {
            let mut sector_buffer = vec![0; bytes_per_sector];
            let bytes_read = f.read(&mut sector_buffer).expect("buffer overflow");
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
            number_of_cells: trackbuf.len() as usize,
            cell_size: PulseDuration(settings.cellsize as u16),
        }];

        tracks.push(RawTrack::new(
            src_cylinder as u32 * 2,
            0,
            trackbuf,
            densitymap,
        ));
    }
    tracks
}
