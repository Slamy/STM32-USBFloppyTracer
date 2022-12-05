use util::{DensityMapEntry, PulseDuration};

use std::convert::TryInto;
use std::fs::{self, File};
use std::io::Read;

use crate::rawtrack::RawTrack;

const G64_SPEED_TABLE: [u32; 4] = [227, 245, 262, 280];

// http://www.unusedino.de/ec64/technical/formats/g64.html

fn u8_buf_to_u32_buf(byte_buffer: &[u8]) -> Vec<u32> {
    let u32_buffer: Vec<u32> = byte_buffer
        .chunks_exact(std::mem::size_of::<u32>())
        .map(|f| u32::from_le_bytes(f.try_into().unwrap()))
        .collect();

    u32_buffer
}

fn auto_cell_size(tracklen: u32) -> f64 {
    let number_cells = tracklen * 8;
    let rpm = 361.0; // Normally 360 RPM would be correct. But the drive might be faster. Let's be safe here.
    let seconds_per_revolution = 60.0 / rpm;
    let microseconds_per_cell = 10_f64.powi(6) * seconds_per_revolution / number_cells as f64;
    let stm_timer_mhz = 84.0;
    let raw_timer_val = stm_timer_mhz * microseconds_per_cell;
    raw_timer_val
}

fn patch_cell_size(file_hash_str: &str, cyl: u8) -> Option<u32> {
    match (file_hash_str, cyl) {
        // Katakis Copy Protection Track must be very precise
        // Usually we would go with a cell size of 245 as it would be the correct timing.
        // But this is too short sometimes!
        // Because of fluctuations in the drives rotation, we need to go higher.
        // But what is the sweet spot? I test the limits and then we go for the average.
        // 247 is the maximum at which the copy protection works.
        // 245 is the minimum at which it sometimes works.
        // Even at 248 the disk is not detected as valid. This protection is very picky.
        // It also greatly depends on the selected RPM in auto_cell_size.
        // I think, In the end I will go for 246 here. It should be a sweet spot.
        ("53c47c575d057181a1911e6653229324", 70) => Some(246),
        _ => None,
    }
}

fn patch_trackdata(source: &[u8], file_hash_str: &str, cyl: u8) -> Vec<u8> {
    match (file_hash_str, cyl) {
        // Various tracks of Katakis have garbage at the end. Remove residual flux data.
        ("53c47c575d057181a1911e6653229324", 50) => source[0..source.len() - 40].into(),
        ("53c47c575d057181a1911e6653229324", 52) => source[0..source.len() - 40].into(),
        ("53c47c575d057181a1911e6653229324", 58) => source[0..source.len() - 40].into(),

        // Katakis Copy Protection Track is too long in this image.
        ("53c47c575d057181a1911e6653229324", 70) => {
            let mut x: Vec<u8> = source[0..source.len() - 300].into();
            x[510] = 0x55;
            x
        }
        // Unused track of the game with impossible to write data. Remove it.
        ("53c47c575d057181a1911e6653229324", 72) => Vec::new(),

        _ => source.into(),
    }
}

pub fn parse_g64_image(path: &str) -> Vec<RawTrack> {
    println!("Reading G64 from {} ...", path);

    let mut f = File::open(&path).expect("no file found");
    let metadata = fs::metadata(&path).expect("unable to read metadata");

    let mut whole_file_buffer: Vec<u8> = vec![0; metadata.len() as usize];
    let bytes_read = f.read(whole_file_buffer.as_mut()).unwrap();
    assert_eq!(bytes_read, metadata.len() as usize);

    let file_hash = md5::compute(&whole_file_buffer);
    let file_hashstr = format!("{:x}", file_hash);

    let (file_header_view, rest_of_file) = whole_file_buffer.split_at(12);

    assert!("GCR-1541".as_bytes().eq(&file_header_view[0..8]));
    let g64_version = file_header_view[8];
    assert_eq!(g64_version, 0);
    let number_of_tracks = file_header_view[9];
    let _size_of_track = u16::from_le_bytes(file_header_view[10..12].try_into().unwrap());

    let (track_offsets_u8, rest_of_file) =
        rest_of_file.split_at(number_of_tracks as usize * std::mem::size_of::<u32>());
    let (speed_offsets_u8, _rest_of_file) =
        rest_of_file.split_at(number_of_tracks as usize * std::mem::size_of::<u32>());

    let track_offsets = u8_buf_to_u32_buf(track_offsets_u8);
    let speed_offsets = u8_buf_to_u32_buf(speed_offsets_u8);

    let mut tracks: Vec<RawTrack> = Vec::new();

    for track_index in 0..number_of_tracks {
        let track_offset = track_offsets[track_index as usize] as usize;
        let speed_offset = 3 - speed_offsets[track_index as usize] as usize;

        let mut cellsize = G64_SPEED_TABLE[speed_offset] as u32;

        if track_offset > 0 {
            let trackdata_copy: Vec<u8>;
            {
                // Don't let actual track size out! trackdata_copy shall solve that with its len
                let actual_track_size = u16::from_le_bytes(
                    whole_file_buffer[track_offset..track_offset + 2]
                        .try_into()
                        .unwrap(),
                ) as usize;

                let trackdata =
                    &whole_file_buffer[track_offset + 2..track_offset + actual_track_size + 2];

                if trackdata.iter().all(|f| *f == 0) {
                    println!("Track {} is all zero? Remove it...", track_index,);
                    continue;
                }

                trackdata_copy = patch_trackdata(trackdata, &file_hashstr, track_index);
                if trackdata_copy.len() == 0 {
                    continue;
                }
            }

            let auto_cell_size = auto_cell_size(trackdata_copy.len() as u32) as u32;

            println!(
                "Track {} Len {:?} cellsize {} auto_cell_size {}",
                track_index,
                trackdata_copy.len(),
                cellsize,
                auto_cell_size
            );

            if auto_cell_size < cellsize {
                println!(
                    "Auto reduce cellsize from {} to {}",
                    cellsize, auto_cell_size
                );
                cellsize = auto_cell_size;
            }

            if let Some(force_track_size) = patch_cell_size(&file_hashstr, track_index) {
                println!(
                    "Force cell size because of patch process from {} to {}",
                    cellsize, force_track_size
                );
                cellsize = force_track_size;
            }

            let densitymap = vec![DensityMapEntry {
                number_of_cells: trackdata_copy.len() as usize,
                cell_size: PulseDuration(cellsize as u16),
            }];

            tracks.push(RawTrack::new(
                track_index as u32,
                0,
                trackdata_copy,
                densitymap,
                util::Encoding::GCR,
            ));
        }
    }

    tracks
}
