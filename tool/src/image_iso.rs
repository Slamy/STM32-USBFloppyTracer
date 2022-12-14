use util::bitstream::BitStreamCollector;
use util::mfm::MfmEncoder;
use util::mfm::MfmWord;
use util::Bit;
use util::Density;
use util::{DensityMapEntry, PulseDuration};

use std::fs::{self, File};
use std::io::Read;
use std::slice::ChunksExact;

use crate::rawtrack::RawImage;
use crate::rawtrack::RawTrack;

// Information sources:
// https://www-user.tu-chemnitz.de/~heha/basteln/PC/usbfloppy/floppy.chm/
// http://info-coach.fr/atari/software/FD-Soft.php

const HEADS: u32 = 2;
const BYTES_PER_SECTOR: u32 = 512;

const POSSIBLE_CYLINDER_COUNTS: [u32; 10] = [38, 39, 40, 41, 42, 78, 79, 80, 81, 82];
const POSSIBLE_SECTOR_COUNTS: [u32; 4] = [9, 10, 11, 18];

fn calculate_floppy_geometry(number_bytes: u32) -> (u32, u32) {
    for cylinders in POSSIBLE_CYLINDER_COUNTS {
        for sectors in POSSIBLE_SECTOR_COUNTS {
            if number_bytes == cylinders * HEADS * BYTES_PER_SECTOR * sectors {
                println!("Disk has {} cylinders and {} sectors!", cylinders, sectors);
                return (cylinders, sectors);
            }
        }
    }
    panic!()
}

struct IsoGeometry {
    sectors_per_track: u32,
    gap1_size: i32,    // after index pulse, 60x 0x4E
    gap2_size: i32,    // 12x 0x00 before sector header
    gap3a_size: i32,   // 22x 0x4E after sector header
    gap3b_size: i32,   // 12x 0x00 before actual data
    gap4_size: i32,    // 40x 0x4E after data
    gap5_size: i32,    // ends the track, not really sure what this value shall be...
    interleaving: u32, // with 0 no interleaving applied
}

impl IsoGeometry {
    fn new(sectors_per_track: u32) -> Self {
        // according to http://info-coach.fr/atari/software/FD-Soft.php
        match sectors_per_track {
            10 => IsoGeometry {
                gap1_size: 60,
                gap2_size: 12,
                gap3a_size: 22,
                gap3b_size: 12,
                gap4_size: 40,
                gap5_size: 20,
                sectors_per_track,
                interleaving: 1,
            },
            11 => IsoGeometry {
                gap1_size: 10,
                gap2_size: 3,
                gap3a_size: 22,
                gap3b_size: 12,
                gap4_size: 1,
                gap5_size: 10,
                sectors_per_track,
                interleaving: 1,
            },
            // standard for 9 and 18
            _ => IsoGeometry {
                gap1_size: 60,
                gap2_size: 12,
                gap3a_size: 22,
                gap3b_size: 12,
                gap4_size: 40,
                gap5_size: 650,
                sectors_per_track,
                interleaving: 0,
            },
        }
    }
}

pub fn generate_iso_sectorheader<T>(
    gap2_size: usize,
    idam_cylinder: u8,
    idam_head: u8,
    idam_sector: u8,
    idam_size: u8,
    encoder: &mut MfmEncoder<T>,
) where
    T: FnMut(Bit),
{
    generate_iso_gap(gap2_size, 0, encoder);
    encoder.feed(MfmWord::SyncWord);
    encoder.feed(MfmWord::SyncWord);
    encoder.feed(MfmWord::SyncWord);

    let sector_header = vec![
        0xfe, // IDAM
        idam_cylinder,
        idam_head,
        idam_sector,
        idam_size,
    ];

    let mut crc = crc16::State::<crc16::CCITT_FALSE>::new();
    crc.update(&vec![0xa1, 0xa1, 0xa1]);
    crc.update(&sector_header);
    let crc16 = crc.get();

    sector_header
        .iter()
        .for_each(|byte| encoder.feed_encoded8(*byte));
    encoder.feed_encoded8((crc16 >> 8) as u8);
    encoder.feed_encoded8((crc16 & 0xff) as u8);
}

pub fn generate_iso_data_header<T>(gap3b_size: usize, encoder: &mut MfmEncoder<T>)
where
    T: FnMut(Bit),
{
    // now the actual data of the sector
    generate_iso_gap(gap3b_size, 0, encoder);
    encoder.feed(MfmWord::SyncWord);
    encoder.feed(MfmWord::SyncWord);
    encoder.feed(MfmWord::SyncWord);
    encoder.feed_encoded8(0xfb);
}

pub fn generate_iso_data_with_crc<T>(sectordata: &[u8], encoder: &mut MfmEncoder<T>)
where
    T: FnMut(Bit),
{
    let mut crc = crc16::State::<crc16::CCITT_FALSE>::new();
    crc.update(&vec![0xa1, 0xa1, 0xa1, 0xfb]);
    crc.update(&sectordata);
    let crc16 = crc.get();

    sectordata
        .iter()
        .for_each(|byte| encoder.feed_encoded8(*byte));
    encoder.feed_encoded8((crc16 >> 8) as u8);
    encoder.feed_encoded8((crc16 & 0xff) as u8);
}

pub fn generate_iso_data_with_broken_crc<T>(sectordata: &[u8], encoder: &mut MfmEncoder<T>)
where
    T: FnMut(Bit),
{
    let mut crc = crc16::State::<crc16::CCITT_FALSE>::new();
    crc.update(&vec![0xa1, 0xa1, 0xa1, 0xfb]);
    crc.update(&sectordata);
    let crc16 = crc.get() + 0x1212; // Destroy CRC

    sectordata
        .iter()
        .for_each(|byte| encoder.feed_encoded8(*byte));
    encoder.feed_encoded8((crc16 >> 8) as u8);
    encoder.feed_encoded8((crc16 & 0xff) as u8);
}

pub fn generate_iso_gap<T>(gap_size: usize, value: u8, encoder: &mut MfmEncoder<T>)
where
    T: FnMut(Bit),
{
    for _ in 0..gap_size {
        encoder.feed_encoded8(value);
    }
}

fn generate_interleaving_table(sectors_per_track: usize, interleaving: usize) -> Vec<usize> {
    let mut interleaving_table = vec![0_usize; sectors_per_track as usize];

    for index in 0..sectors_per_track {
        let target = (index * (interleaving + 1)) % sectors_per_track;
        interleaving_table[target as usize] = index;
    }

    interleaving_table
}

fn generate_iso_track(
    cylinder: u32,
    head: u32,
    geometry: &IsoGeometry,
    sectors_in: &mut ChunksExact<u8>,
) -> Vec<u8> {
    let mut trackbuf: Vec<u8> = Vec::new();
    let mut collector = BitStreamCollector::new(|f| trackbuf.push(f));
    let mut encoder = MfmEncoder::new(|cell| collector.feed(cell));

    let mut sectors: Vec<(u8, &[u8])> = Vec::new();
    for sector in 0..geometry.sectors_per_track {
        let sectordata = sectors_in.next().unwrap();
        sectors.push((sector as u8 + 1, sectordata));
    }

    let interleaving_table = generate_interleaving_table(
        geometry.sectors_per_track as usize,
        geometry.interleaving as usize,
    );

    // just after the index pulse
    generate_iso_gap(geometry.gap1_size as usize, 0x4e, &mut encoder);

    for index in interleaving_table {
        let (idam_sector, sectordata) = sectors[index];
        println!("Sector {}", idam_sector);

        // sector header
        generate_iso_sectorheader(
            geometry.gap2_size as usize,
            cylinder as u8,
            head as u8,
            idam_sector,
            2,
            &mut encoder,
        );

        // the gap between sector header and data
        generate_iso_gap(geometry.gap3a_size as usize, 0x4e, &mut encoder);
        generate_iso_data_header(geometry.gap3b_size as usize, &mut encoder);
        generate_iso_data_with_crc(&sectordata, &mut encoder);

        // gap after the sector
        generate_iso_gap(geometry.gap4_size as usize, 0x4e, &mut encoder);
    }
    // end the track
    generate_iso_gap(geometry.gap5_size as usize, 0x4e, &mut encoder);

    trackbuf
}

pub fn parse_iso_image(path: &str) -> RawImage {
    println!("Reading ISO image from {} ...", path);

    let mut f = File::open(&path).expect("no file found");
    let metadata = fs::metadata(&path).expect("unable to read metadata");

    let (cylinders, sectors_per_track) = calculate_floppy_geometry(metadata.len() as u32);

    let geometry = IsoGeometry::new(sectors_per_track);

    let (cellsize, density) = if sectors_per_track == 18 {
        (84, Density::High)
    } else {
        (168, Density::SingleDouble)
    };

    let mut buffer = vec![0; metadata.len() as usize];

    let bytes_read = f.read(&mut buffer).expect("buffer overflow");
    assert!(bytes_read == metadata.len() as usize);

    let mut sectors = buffer.chunks_exact(BYTES_PER_SECTOR as usize);
    let mut tracks: Vec<RawTrack> = Vec::new();

    for cylinder in 0..cylinders {
        for head in 0..HEADS {
            let trackbuf = generate_iso_track(cylinder, head, &geometry, &mut sectors);

            let densitymap = vec![DensityMapEntry {
                number_of_cellbytes: trackbuf.len() as usize,
                cell_size: PulseDuration(cellsize),
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

    RawImage {
        tracks,
        disk_type: util::DiskType::Inch3_5,
        density,
    }
}
