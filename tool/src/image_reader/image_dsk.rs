use std::convert::TryInto;
use std::io::Cursor;
use std::{
    fs::{self, File},
    io::Read,
};

use anyhow::{bail, ensure, Context};
use byteorder::{LittleEndian, ReadBytesExt};
use util::bitstream::BitStreamCollector;
use util::mfm::MfmEncoder;
use util::{Density, DensityMapEntry, PulseDuration, DRIVE_3_5_RPM};

use crate::image_reader::image_iso::{
    generate_iso_data_header, generate_iso_data_with_crc, generate_iso_gap,
    generate_iso_sectorheader, IsoGeometry, ISO_DDAM,
};
use crate::rawtrack::{auto_cell_size, RawImage, RawTrack};

const FDC_765_STAT2_CONTROL_MARK: u8 = 1 << 6;

// info from https://www.cpcwiki.eu/index.php/Format:DSK_disk_image_file_format
// additional info https://simonowen.com/misc/extextdsk.txt
// info about protections of games https://www.cpc-power.com/index.php?page=protection

pub fn parse_dsk_image(path: &str) -> anyhow::Result<RawImage> {
    println!("Reading DSK from {path} ...");

    let mut file = File::open(path)?;
    let metadata = fs::metadata(path)?;

    let mut whole_file_buffer: Vec<u8> = vec![0; metadata.len() as usize];
    let bytes_read = file.read(whole_file_buffer.as_mut())?;
    ensure!(bytes_read == metadata.len() as usize);

    let mut tracks: Vec<RawTrack> = Vec::new();

    let disc_information_block = &ensure_index!(whole_file_buffer[0..256]);

    let type_str = std::str::from_utf8(&ensure_index!(disc_information_block[0..34]))?;

    // Check file type
    let extended = match type_str {
        "MV - CPCEMU Disk-File\r\nDisk-Info\r\n" => false,
        "EXTENDED CPC DSK File\r\nDisk-Info\r\n" => true,
        _ => bail!("DSK File not in expected format!"),
    };

    let number_of_cylinders = ensure_index!(disc_information_block[0x30]) as usize;
    let number_of_sides = ensure_index!(disc_information_block[0x31]) as usize;
    let number_of_tracks = number_of_cylinders * number_of_sides;

    // The track size table only exists with the extended variant of this format
    let track_size_table = if extended {
        Some(&ensure_index!(
            disc_information_block[0x34..(0x34 + number_of_tracks)]
        ))
    } else {
        None
    };
    // Size of track can be safely ignored as it seems.
    let _size_of_track =
        u16::from_le_bytes(ensure_index!(disc_information_block[0x32..0x34]).try_into()?);

    // The first "Track Information Block" starts at offset 0x100 in file
    let mut file_offset = 0x100;

    for track_index in 0..number_of_tracks {
        // Get next "Track Information Block"
        let track_information_block = &ensure_index!(whole_file_buffer[file_offset..]);

        // If a track has zero size, it is unformatted. Just skip it and continue
        if let Some(table) = track_size_table && ensure_index!(table[track_index]) == 0 {
            // TODO better solution for this
            continue;
        }

        // Ensure that we are actually reading the data we expect here
        ensure!(b"Track-Info\r\n".eq(&ensure_index!(track_information_block[0..12])));

        let mut track_info_reader = Cursor::new(&ensure_index!(track_information_block[0x10..]));

        let track_number = track_info_reader.read_u8()?;
        let side_number = track_info_reader.read_u8()?;
        let _unused = track_info_reader.read_u16::<LittleEndian>()?;
        let _sector_size = track_info_reader.read_u8()?;
        let number_of_sectors = track_info_reader.read_u8()? as usize;
        let _gap3_length = track_info_reader.read_u8()?;
        let _filler_byte = track_info_reader.read_u8()?;

        let mut trackbuf: Vec<u8> = Vec::new();
        let mut collector = BitStreamCollector::new(|f| trackbuf.push(f));
        let mut encoder = MfmEncoder::new(|cell| collector.feed(cell));

        let mut sector_info_reader = Cursor::new(&ensure_index!(track_information_block[0x18..]));

        // The first sector starts 0x100 byte after the header information
        file_offset += 0x100;

        let geometry = IsoGeometry::new(number_of_sectors);

        generate_iso_gap(geometry.gap1_size as usize, 0x4e, &mut encoder);

        for _ in 0..number_of_sectors {
            // Get Sector Info
            let sector_track = sector_info_reader.read_u8()?;
            let sector_side = sector_info_reader.read_u8()?;
            let sector_id = sector_info_reader.read_u8()?;
            let sector_size = sector_info_reader.read_u8()?;
            let _fdc_status1 = sector_info_reader.read_u8()?;
            let fdc_status2 = sector_info_reader.read_u8()?;

            // In case of the extended format one additional field is added which stores
            // the actual size of the sector. This is important for Sectors of size 6
            // which are used for the Hexagon Protection
            let actual_data_length = if extended {
                sector_info_reader.read_u16::<LittleEndian>()? as usize
            } else {
                sector_info_reader.read_u16::<LittleEndian>()?; //unused
                128 << sector_size
            };

            let sector_data =
                &ensure_index!(whole_file_buffer[file_offset..(file_offset + actual_data_length)]);

            file_offset += actual_data_length;

            // TODO I guess this is debatable. I want to find the next sector. But how shall it be done correctly?
            // This works for now... I hope
            if file_offset & 0xff != 0 {
                file_offset = (file_offset | 0xff) + 1;
            }

            generate_iso_sectorheader(
                geometry.gap2_size as usize,
                sector_track,
                sector_side,
                sector_id,
                sector_size,
                &mut encoder,
            );
            generate_iso_gap(geometry.gap3a_size as usize, 0x4e, &mut encoder);

            // Some protections use sectors which are marked as deleted.
            let address_mark = if (fdc_status2 & FDC_765_STAT2_CONTROL_MARK) != 0 {
                Some(ISO_DDAM) // deleted data
            } else {
                None // use standard address mark
            };
            generate_iso_data_header(geometry.gap3b_size as usize, &mut encoder, address_mark);
            generate_iso_data_with_crc(sector_data, &mut encoder, address_mark);
            // gap after the sector
            generate_iso_gap(geometry.gap4_size as usize, 0x4e, &mut encoder);
        }

        // end the track
        generate_iso_gap(geometry.gap5_size as usize, 0x4e, &mut encoder);

        let auto_cell_size = auto_cell_size(trackbuf.len() as u32, DRIVE_3_5_RPM).min(168.0_f64);

        let densitymap = vec![DensityMapEntry {
            number_of_cellbytes: trackbuf.len(),
            cell_size: PulseDuration(auto_cell_size as i32),
        }];

        tracks.push(RawTrack::new(
            u32::from(track_number),
            u32::from(side_number),
            trackbuf,
            densitymap,
            util::Encoding::MFM,
        ));
    }

    Ok(RawImage {
        tracks,
        disk_type: util::DiskType::Inch3_5,
        density: Density::SingleDouble,
    })
}
