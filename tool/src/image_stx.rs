use crate::image_iso::{
    generate_iso_data_header, generate_iso_data_with_crc, generate_iso_gap,
    generate_iso_sectorheader,
};
use crate::rawtrack::{auto_cell_size, RawImage, RawTrack, DRIVE_3_5_RPM};
use std::cell::RefCell;
use std::fs::{self, File};
use std::io::Cursor;
use std::io::Read;
use util::bitstream::BitStreamCollector;
use util::mfm::{MfmEncoder, MfmWord};
use util::{Bit, Density, DensityMapEntry, PulseDuration};

// Information source:
// http://info-coach.fr/atari/documents/_mydoc/Pasti-documentation.pdf
// https://info-coach.fr/atari/documents/_mydoc/Atari-Copy-Protection.pdf
// https://github.com/sarnau/AtariSTCopyProtections/blob/master/protection_turrican.md

const _TRK_SYNC: u16 = 0x80; // track image header contains sync offset info
const _TRK_IMAGE: u16 = 0x40; // track record contains track image
const _TRK_PROT: u16 = 0x20; // track contains protections ? not used?
const TRK_SECT: u16 = 0x01; // track record contains sector descriptor

use byteorder::{BigEndian, LittleEndian, ReadBytesExt};

struct StxSector {
    data_offset: usize,
    bit_position: usize,
    _read_time: u16,
    idam_track: u8,
    idam_head: u8,
    idam_sector: u8,
    idam_size: u8,
    idam_crc: u16,
    fdc_flags: u8,
    sector_size: usize,
}

const gap2_size: usize = 3; // Minimal allowed preamble with 0x00 before sector header
const gap3a_size: usize = 22; // Minimal allowed gap between sector header and data (0x4E)
const gap3b_size: usize = 12; // 12x 0x00 before actual data

fn patch_discard_sector(sector: &StxSector, file_hash_str: &str) -> bool {
    match (file_hash_str, sector.idam_sector) {
        ("4865957cd83562547a722c95e9a5421a", 16) => {
            // Part of copy protection of Turrican
            // Remove this sector as it is inside the data of sector 0!
            true
        }
        _ => false, // Use the sector normally
    }
}

fn patch_custom_sector<T>(
    sector: &StxSector,
    file_hash_str: &str,
    encoder: &mut MfmEncoder<T>,
) -> bool
where
    T: FnMut(Bit),
{
    match (file_hash_str, sector.idam_sector) {
        ("4865957cd83562547a722c95e9a5421a", 0) => {
            // Ok, this is ugly.
            // The copy protection of Turrican is not recorded in the STX file very well.
            // STX files only contain the view on the data based on what the floppy controller has seen.
            // We must reconstruct the original data here
            // This is based on
            // https://info-coach.fr/atari/documents/_mydoc/Atari-Copy-Protection.pdf
            // https://github.com/sarnau/AtariSTCopyProtections/blob/master/protection_turrican.md
            // I'm very thankful for these infos.
            generate_iso_sectorheader(
                gap2_size,
                sector.idam_track,
                sector.idam_head,
                0,
                sector.idam_size,
                encoder,
            );

            // the gap between sector header and data
            generate_iso_gap(gap3a_size, 0x4e, encoder);
            generate_iso_data_header(gap3b_size, encoder);

            generate_iso_sectorheader(
                16,
                sector.idam_track,
                sector.idam_head,
                16,
                sector.idam_size,
                encoder,
            );
            generate_iso_gap(22, 0x4e, encoder);

            // shift the data to allow reading data bits using sector 16
            // and clock bits by reading sector 0.
            // this is insane
            encoder.feed_raw_var(0x5555 >> 1, 15);

            generate_iso_data_header(11, encoder);

            // actual data which is 0x00 in sector 16 but 0xff in sector 0
            generate_iso_gap(16, 0x00, encoder);
            encoder.feed_raw_var(0xa000, 16);

            // produce no flux reversal area.
            for _ in 0..262 {
                encoder.feed_raw8(0);
            }

            // TODO: For yet unknown reasons, the data after the no flux reversal ara
            // is not allowed to be any data. I need to check why this is the case...
            // but for know this doesn't hurt.
            encoder.feed_raw8(0b10101010);
            true
        }
        _ => false, // No patch? Just return false to indicate that a normal sector shall be generated
    }
}

pub fn parse_stx_image(path: &str) -> RawImage {
    println!("Reading STX from {} ...", path);

    let mut f = File::open(&path).expect("no file found");
    let metadata = fs::metadata(&path).expect("unable to read metadata");

    let mut whole_file_buffer: Vec<u8> = vec![0; metadata.len() as usize];
    let bytes_read = f.read(whole_file_buffer.as_mut()).unwrap();
    assert_eq!(bytes_read, metadata.len() as usize);

    let file_hash = md5::compute(&whole_file_buffer);
    let file_hash_str = format!("{:x}", file_hash);

    assert!(
        "RSY\0".as_bytes().eq(&whole_file_buffer[0..4]),
        "Is this really an STX / Pasti file?"
    );

    let mut file_desc_reader = Cursor::new(&whole_file_buffer[4..]);

    let version = file_desc_reader.read_u16::<LittleEndian>().unwrap();
    let _tool = file_desc_reader.read_u16::<LittleEndian>().unwrap();
    let _reserved1 = file_desc_reader.read_u16::<LittleEndian>().unwrap();
    let track_count = file_desc_reader.read_u8().unwrap();
    let _revision = file_desc_reader.read_u8().unwrap();
    let _reserved2 = file_desc_reader.read_u32::<LittleEndian>().unwrap();

    assert_eq!(version, 3);
    println!("Number of tracks {}", track_count);

    let mut current_track_record_position = 16;

    let mut tracks: Vec<RawTrack> = Vec::new();

    for _ in 0..track_count {
        let mut has_non_flux_reversal_area = false;

        let mut track_record_reader =
            Cursor::new(&whole_file_buffer[current_track_record_position..]);
        let record_size = track_record_reader.read_u32::<LittleEndian>().unwrap() as usize;
        let fuzzy_count = track_record_reader.read_u32::<LittleEndian>().unwrap();
        let sector_count = track_record_reader.read_u16::<LittleEndian>().unwrap();
        let track_flags = track_record_reader.read_u16::<LittleEndian>().unwrap();
        let track_length = track_record_reader.read_u16::<LittleEndian>().unwrap() as usize;
        let track_number = track_record_reader.read_u8().unwrap();
        let _track_type = track_record_reader.read_u8().unwrap();

        assert_eq!(fuzzy_count, 0, "Fuzzy not supported yet!");

        const SECTOR_DESCRIPTOR_SIZE: usize = 16;
        const TRACK_DESCRIPTOR_SIZE: usize = 16;

        // Track data contains the "Optional Track Image" and the "Optional Sector Images"
        // The "Optional Fuzzy Mask" is yet ignored here.
        let track_data_start = current_track_record_position
            + TRACK_DESCRIPTOR_SIZE
            + SECTOR_DESCRIPTOR_SIZE * sector_count as usize;

        // For the Track Data End, the "Optional Timing" is yet ignored.
        let track_data_end = record_size + current_track_record_position;

        let track_data = &whole_file_buffer[track_data_start..track_data_end];

        let cylinder = track_number & 0x7f;
        let head = track_number >> 7;

        if sector_count != 0 {
            assert!((track_flags & TRK_SECT) != 0);

            let mut byte_position_offset = None;
            let mut sectors: Vec<StxSector> = Vec::new();

            for _ in 0..sector_count {
                let data_offset = track_record_reader.read_u32::<LittleEndian>().unwrap() as usize;
                let bit_position = track_record_reader.read_u16::<LittleEndian>().unwrap() as usize;
                let _read_time = track_record_reader.read_u16::<LittleEndian>().unwrap();

                let idam_track = track_record_reader.read_u8().unwrap();
                let idam_head = track_record_reader.read_u8().unwrap();
                let idam_sector = track_record_reader.read_u8().unwrap();
                let idam_size = track_record_reader.read_u8().unwrap();
                let idam_crc = track_record_reader.read_u16::<BigEndian>().unwrap();

                let fdc_flags = track_record_reader.read_u8().unwrap();
                let _reserved = track_record_reader.read_u8().unwrap();

                let sector_size = 128 << idam_size;

                sectors.push(StxSector {
                    data_offset,
                    bit_position,
                    _read_time,
                    idam_track,
                    idam_head,
                    idam_sector,
                    idam_size,
                    idam_crc,
                    fdc_flags,
                    sector_size,
                });
            }

            sectors.sort_by(|a, b| a.bit_position.cmp(&b.bit_position));

            let trackbuf: RefCell<Vec<u8>> = RefCell::new(Vec::new());
            let mut collector = BitStreamCollector::new(|f| trackbuf.borrow_mut().push(f));
            let mut encoder = MfmEncoder::new(|cell| collector.feed(cell));

            for sector in sectors.iter() {
                if patch_discard_sector(sector, &file_hash_str) {
                    continue;
                }

                assert!(sector.idam_head < 2);

                if byte_position_offset.is_none() {
                    if sectors.len() == 1 {
                        byte_position_offset = Some(0);
                    } else {
                        byte_position_offset = Some(sector.bit_position / 4);
                    }
                }

                let mfm_word_position = sector.bit_position / 4 - byte_position_offset.unwrap();
                let dynamic_gap_size =
                    (mfm_word_position as i32 - trackbuf.borrow().len() as i32) / 2;

                if dynamic_gap_size >= 0 {
                    for _ in 0..dynamic_gap_size {
                        encoder.feed_encoded8(0x4e);
                    }
                }

                let custom_sector = patch_custom_sector(sector, &file_hash_str, &mut encoder);

                if custom_sector {
                    // TODO really the right approach?
                    has_non_flux_reversal_area = true;
                }

                if custom_sector == false {
                    // No special code required to fix this image? Just do it normally

                    let sector_data =
                        &track_data[sector.data_offset..(sector.data_offset + sector.sector_size)];

                    // sector header
                    generate_iso_gap(gap2_size, 0, &mut encoder);

                    // usually we would have a function for this. but STX is rather special
                    // as this code allows wrong sector header CRCs as STX files support that.
                    encoder.feed(MfmWord::SyncWord);
                    encoder.feed(MfmWord::SyncWord);
                    encoder.feed(MfmWord::SyncWord);

                    let sector_header = vec![
                        0xfe, // IDAM
                        sector.idam_track,
                        sector.idam_head,
                        sector.idam_sector,
                        sector.idam_size,
                        (sector.idam_crc >> 8) as u8,
                        (sector.idam_crc & 0xff) as u8,
                    ];
                    sector_header
                        .iter()
                        .for_each(|byte| encoder.feed_encoded8(*byte));

                    // gap between sector header and sector data
                    for _ in 0..gap3a_size {
                        encoder.feed_encoded8(0x4e);
                    }

                    // now the actual data of the sector
                    generate_iso_data_header(gap3b_size, &mut encoder);

                    if sector.fdc_flags == 0 {
                        generate_iso_data_with_crc(&sector_data, &mut encoder);
                    } else if sector.fdc_flags == 8 {
                        panic!("CRC Error ? Don't know what to do here!");
                    } else {
                        panic!("Unsupported set of fdc flags: {}", sector.fdc_flags);
                    }
                }
            }

            let dynamic_gap5_size = (track_length * 2 - trackbuf.borrow().len()) / 2;

            // end the track
            for _ in 0..dynamic_gap5_size {
                encoder.feed_encoded8(0x4e);
            }

            assert!(
                track_length * 2 >= trackbuf.borrow().len(),
                "trackbuf too long!"
            );

            let cellsize = auto_cell_size(trackbuf.borrow().len() as u32, DRIVE_3_5_RPM) as i32;

            let densitymap = vec![DensityMapEntry {
                number_of_cells: trackbuf.borrow().len() as usize,
                cell_size: PulseDuration(cellsize),
            }];

            let track = RawTrack::new_with_non_flux_reversal_area(
                cylinder as u32,
                head as u32,
                trackbuf.take(),
                densitymap,
                util::Encoding::MFM,
                has_non_flux_reversal_area,
            );

            tracks.push(track);
        }

        current_track_record_position += record_size;
    }

    tracks.sort_by(|a, b| a.cylinder.cmp(&b.cylinder));

    RawImage {
        tracks,
        disk_type: util::DiskType::Inch3_5,
        density: Density::SingleDouble,
    }
}
