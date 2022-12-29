use crate::rawtrack::{auto_cell_size, RawImage, RawTrack};
use std::convert::TryInto;
use std::fs::{self, File};
use std::io::Read;
use util::{DensityMapEntry, PulseDuration, DRIVE_5_25_RPM};

const G64_SPEED_TABLE: [u32; 4] = [227, 245, 262, 280];

// http://www.unusedino.de/ec64/technical/formats/g64.html

fn u8_buf_to_u32_buf(byte_buffer: &[u8]) -> Vec<u32> {
    let u32_buffer: Vec<u32> = byte_buffer
        .chunks_exact(std::mem::size_of::<u32>())
        .map(|f| u32::from_le_bytes(f.try_into().unwrap()))
        .collect();

    u32_buffer
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
        ("d2aa92ccf3531fc995e771be91a45241", 70) => Some(246),
        ("406d29151e7001f6bfc7d95b7ade799d", 70) => Some(246),

        // "Great Giana Sisters" Copy Protection Track
        // Set timing like Katakis to be sure
        ("c2334233136c523b9ec62beb8bea1e00", 70) => Some(246),

        _ => None,
    }
}

fn patch_trackdata(source: &[u8], file_hash_str: &str, cyl: u8) -> Vec<u8> {
    match (file_hash_str, cyl) {
        // Katakis Copy Protection Track is too long in this image.
        ("53c47c575d057181a1911e6653229324", 70) => {
            let x: Vec<u8> = source[0..source.len() - 300].into();
            x
        }
        ("d2aa92ccf3531fc995e771be91a45241", 70) => {
            let mut x: Vec<u8> = source[0..source.len() - 300].into();
            x[0..0x22b].fill(0x55);
            x[0x22b] = 0x57;
            x[0x22c..0x2ac].fill(0xff);
            x
        }
        ("406d29151e7001f6bfc7d95b7ade799d", 70) => {
            let mut x: Vec<u8> = source[0..source.len() - 300].into();
            x[0x22c..0x2ac].fill(0xff);
            x
        }

        // Unused track of the game with impossible to write data. Remove it.
        ("53c47c575d057181a1911e6653229324", 72) => Vec::new(),
        ("d2aa92ccf3531fc995e771be91a45241", 72) => Vec::new(),
        ("406d29151e7001f6bfc7d95b7ade799d", 72) => Vec::new(),

        // "Great Giana Sisters" Copy Protection Track is too long in this image.
        ("c2334233136c523b9ec62beb8bea1e00", 70) => {
            let x: Vec<u8> = source[0..source.len() - 1000].into();
            x
        }
        ("c2334233136c523b9ec62beb8bea1e00", 72) => Vec::new(),

        _ => source.into(),
    }
}

pub fn parse_g64_image(path: &str) -> RawImage {
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

            let auto_cell_size = auto_cell_size(trackdata_copy.len() as u32, DRIVE_5_25_RPM) as u32;

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
                number_of_cellbytes: trackdata_copy.len() as usize,
                cell_size: PulseDuration(cellsize as i32),
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

    RawImage {
        tracks,
        disk_type: util::DiskType::Inch5_25,
        density: util::Density::SingleDouble,
    }
}
