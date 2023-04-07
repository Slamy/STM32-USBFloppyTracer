use super::image_iso::{
    generate_iso_data_header, generate_iso_data_with_broken_crc, generate_iso_data_with_crc,
    generate_iso_gap, generate_iso_sectorheader,
};
use crate::image_reader::image_iso::{ISO_DAM, ISO_IDAM};
use crate::rawtrack::{RawImage, RawTrack};
use anyhow::{ensure, Context};
use std::cell::RefCell;
use std::fs::{self, File};
use std::io::Cursor;
use std::io::Read;
use util::bitstream::BitStreamCollector;
use util::mfm::{MfmEncoder, MfmWord, ISO_SYNC_BYTE};
use util::{
    reduce_densitymap, Bit, Density, DensityMap, DensityMapEntry, PulseDuration, STM_TIMER_HZ,
};

// Information source:
// http://info-coach.fr/atari/documents/_mydoc/Pasti-documentation.pdf
// https://info-coach.fr/atari/documents/_mydoc/Atari-Copy-Protection.pdf
// https://github.com/sarnau/AtariSTCopyProtections/blob/master/protection_turrican.md

const TRK_SYNC: u16 = 0x80; // track image header contains sync offset info
const TRK_IMAGE: u16 = 0x40; // track record contains track image
const _TRK_PROT: u16 = 0x20; // track contains protections ? not used?
const TRK_SECT: u16 = 0x01; // track record contains sector descriptor

const _FDC_FLAG_FUZZY_MASK_RECORD: u8 = 1 << 7;
const _FDC_FLAG_DELETED_DATA: u8 = 1 << 5;
const FDC_FLAG_RECORD_NOT_FOUND: u8 = 1 << 4;
const FDC_FLAG_CRC_ERROR: u8 = 1 << 3;
const FDC_FLAG_INTRA_SECTOR_BIT_WIDTH_VARIATION: u8 = 1; // Macrodos / Speedlock

use byteorder::{BigEndian, LittleEndian, ReadBytesExt};

struct StxSector {
    data_offset: usize,
    bit_position: usize,
    read_time: u32,
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
    matches!(
        (file_hash_str, sector.idam_sector),
        ("4865957cd83562547a722c95e9a5421a", 16)
    )
}

fn patch_custom_sector<T>(
    sector: &StxSector,
    file_hash_str: &str,
    encoder: &mut MfmEncoder<T>,
    has_non_flux_reversal_area: &mut bool,
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
            generate_iso_data_header(gap3b_size, encoder, None);

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

            generate_iso_data_header(11, encoder, None);

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
            encoder.feed_raw8(0b1010_1010);

            *has_non_flux_reversal_area = true;
            true
        }
        _ => false, // No patch? Just return false to indicate that a normal sector shall be generated
    }
}

fn read_time_to_cellsize_in_seconds(sector_read_time: u16, sector_size: usize) -> f64 {
    1e-6 * f64::from(sector_read_time) / (sector_size * 16) as f64
}

#[derive(Clone, Debug)]
pub struct SectorTimingDeviation {
    pub number_of_raw_bytes: usize,
    pub cell_size_in_seconds: f64,
}

fn total_number_raw_bytes_deviation_map(deviation_map: &[SectorTimingDeviation]) -> usize {
    deviation_map.iter().map(|f| f.number_of_raw_bytes).sum()
}

const SECTOR_DESCRIPTOR_SIZE: usize = 16;
const TRACK_DESCRIPTOR_SIZE: usize = 16;

pub fn parse_stx_image(path: &str) -> anyhow::Result<RawImage> {
    println!("Reading STX from {path} ...");

    let mut f = File::open(path)?;
    let metadata = fs::metadata(path)?;

    let mut whole_file_buffer: Vec<u8> = vec![0; metadata.len() as usize];
    let bytes_read = f.read(whole_file_buffer.as_mut())?;
    ensure!(bytes_read == metadata.len() as usize);

    let file_hash = md5::compute(&whole_file_buffer);
    let file_hash_str = format!("{file_hash:x}");

    ensure!(
        b"RSY\0".eq(&ensure_index!(whole_file_buffer[0..4])),
        "Is this really an STX / Pasti file?"
    );

    // --- Reading File Descriptor ---
    let mut file_desc_reader = Cursor::new(&ensure_index!(whole_file_buffer[4..]));

    let version = file_desc_reader.read_u16::<LittleEndian>()?;
    let _tool = file_desc_reader.read_u16::<LittleEndian>()?;
    let _reserved1 = file_desc_reader.read_u16::<LittleEndian>()?;
    let track_count = file_desc_reader.read_u8()?;
    let revision = file_desc_reader.read_u8()?;
    let _reserved2 = file_desc_reader.read_u32::<LittleEndian>()?;

    ensure!(version == 3, "Only Pasti version 3 is supported!");
    println!("Number of tracks {track_count}, File Revision {revision}");

    // After the File Descriptor follows the track records
    let mut current_track_record_position = 16;

    let mut tracks: Vec<RawTrack> = Vec::new();

    // Iterate over all track records
    for _ in 0..track_count {
        let (optional_track, next_track_record_offset) = process_track_record(
            &whole_file_buffer,
            current_track_record_position,
            &file_hash_str,
            revision,
        )?;

        if let Some(track) = optional_track {
            tracks.push(track);
        }

        current_track_record_position = next_track_record_offset;
    }

    tracks.sort_by_key(|a| a.cylinder);

    Ok(RawImage {
        tracks,
        disk_type: util::DiskType::Inch3_5,
        density: Density::SingleDouble,
    })
}

fn read_sector_descriptors(
    sector_count: usize,
    track_record_reader: &mut Cursor<&[u8]>,
) -> anyhow::Result<(Vec<StxSector>, usize)> {
    // We could process the sector descriptors during the reading process.
    // But if we store them first and use them later, we can perform
    // post processing tasks. For example we can change the order or drop sectors.
    let mut sectors: Vec<StxSector> = Vec::new();
    let mut timing_data_size: usize = 0;

    for _ in 0..sector_count {
        // Read a Sector Descriptor
        let data_offset = track_record_reader.read_u32::<LittleEndian>()? as usize;
        let bit_position = track_record_reader.read_u16::<LittleEndian>()? as usize;
        let read_time = u32::from(track_record_reader.read_u16::<LittleEndian>()?);

        let idam_track = track_record_reader.read_u8()?;
        let idam_head = track_record_reader.read_u8()?;
        let idam_sector = track_record_reader.read_u8()?;
        let idam_size = track_record_reader.read_u8()?;
        let idam_crc = track_record_reader.read_u16::<BigEndian>()?;

        let fdc_flags = track_record_reader.read_u8()?;
        let _reserved = track_record_reader.read_u8()?;
        let sector_size = 128 << idam_size;

        if (fdc_flags & FDC_FLAG_INTRA_SECTOR_BIT_WIDTH_VARIATION) != 0 {
            // for 16 bytes of sector data we have 2 bytes of timing data
            timing_data_size += sector_size / 8;
        }

        ensure!(idam_head < 2);

        ensure!(fdc_flags & (1 << 5) == 0, "Deleted data not yet supported");

        sectors.push(StxSector {
            data_offset,
            bit_position,
            read_time,
            idam_track,
            idam_head,
            idam_sector,
            idam_size,
            idam_crc,
            fdc_flags,
            sector_size,
        });
    }

    // Some images have their sector order shifted.
    // Sort them by the bit_position which marks the position of the sector on disk.
    // For an emulator this is not important but we are writing a track here from start
    // to finish in one sitting.
    sectors.sort_by_key(|a| a.bit_position);

    Ok((sectors, timing_data_size))
}

fn read_timing_record(optional_timing_record: &[u8]) -> anyhow::Result<Vec<f64>> {
    println!("timing sector {optional_timing_record:x?}");

    let mut timing_record_reader = Cursor::new(&optional_timing_record);
    let flags = timing_record_reader.read_u16::<LittleEndian>()?;
    let timing_record_size = timing_record_reader.read_u16::<LittleEndian>()? as usize;
    let timing_data_size = timing_record_size - 4;

    ensure!(flags == 5, "Unexpected flags in timing descriptor");
    ensure!(
        timing_record_size == optional_timing_record.len(),
        "Timing record sizes don't match!"
    );

    let mut timing_data = Vec::new();
    for _ in 0..timing_data_size / 2 {
        // the timing value is defined as the microseconds *4 it takes to read 16 data bytes
        // the nominal value is 128, which is 512 microseconds (16 * microseconds per data byte)
        let timing_value = timing_record_reader.read_u16::<BigEndian>()?;
        let cellsize_in_seconds = 1e-6 * f64::from(timing_value) / 64.0;
        //let raw_cellsize = (cellsize_microseconds * 84.0).round() as u16;
        timing_data.push(cellsize_in_seconds);
    }

    println!("{} {:?}", timing_data.len(), timing_data);
    Ok(timing_data)
}

fn convert_timing_deviation_to_densitymap(
    mut deviation_map: Vec<SectorTimingDeviation>,
) -> anyhow::Result<DensityMap> {
    // now the deviation map should have the same number of raw bytes as the track buffer contains.
    let deviation_map_total_time: f64 = deviation_map
        .iter()
        .map(|f| f.cell_size_in_seconds * f.number_of_raw_bytes as f64 * 8.0)
        .sum();

    let one_rotation_in_seconds = 0.1999; // little bit less than 200ms to be safe.

    // does our current data fit into one single rotation of the disk?
    if deviation_map_total_time > one_rotation_in_seconds {
        // No it doesn't. We need to fix this a bit.
        // The reason for this is that the read time doesn't contain the gaps.

        let correction_factor = one_rotation_in_seconds / deviation_map_total_time;
        ensure!(
            correction_factor > 0.99,
            "Correction factor {} not plausible",
            correction_factor
        );

        deviation_map
            .iter_mut()
            .for_each(|f| f.cell_size_in_seconds *= correction_factor);
    }

    // Now we create a densitymap from the deviation data and finally get values
    // usable by the usb device.
    let densitymap: DensityMap = deviation_map
        .iter()
        .map(|f| DensityMapEntry {
            cell_size: PulseDuration((f.cell_size_in_seconds * STM_TIMER_HZ) as i32),
            number_of_cellbytes: f.number_of_raw_bytes,
        })
        .collect();

    Ok(reduce_densitymap(densitymap))
}

fn process_track_record(
    whole_file_buffer: &[u8],
    current_track_record_position: usize,
    file_hash_str: &str,
    revision: u8,
) -> anyhow::Result<(Option<RawTrack>, usize)> {
    let mut has_non_flux_reversal_area = false;

    // Read Track Descriptor
    let mut track_record_reader = Cursor::new(&ensure_index!(
        whole_file_buffer[current_track_record_position..]
    ));

    let record_size = track_record_reader.read_u32::<LittleEndian>()? as usize;
    let fuzzy_count = track_record_reader.read_u32::<LittleEndian>()? as usize;
    let sector_count = track_record_reader.read_u16::<LittleEndian>()? as usize;
    let track_flags = track_record_reader.read_u16::<LittleEndian>()?;
    let track_length = track_record_reader.read_u16::<LittleEndian>()? as usize;
    let track_number = track_record_reader.read_u8()?;
    let _track_type = track_record_reader.read_u8()?;

    let optional_fuzzy_mask_start = current_track_record_position
        + TRACK_DESCRIPTOR_SIZE
        + SECTOR_DESCRIPTOR_SIZE * sector_count;

    // Track data contains the "Optional Track Image" and the "Optional Sector Images"
    let track_data_start = optional_fuzzy_mask_start + fuzzy_count;

    let next_track_record_offset = record_size + current_track_record_position;

    // Bit 7 of the track number contains the side of the disk.
    // The lower 7 bits contain the cylinder.
    let cylinder = track_number & 0x7f;
    let head = track_number >> 7;

    let (sectors, timing_data_size) =
        read_sector_descriptors(sector_count, &mut track_record_reader)?;

    let optional_timing_record_size = if timing_data_size > 0 {
        ensure!(
            revision == 2,
            "Revision != 2 is not supported with intra sector bit width variation!"
        );

        timing_data_size + 4
    } else {
        0
    };

    let track_data_end = next_track_record_offset - optional_timing_record_size;
    let track_data = &ensure_index!(whole_file_buffer[track_data_start..track_data_end]);

    if fuzzy_count > 0 {
        let _fuzzy_mask =
            &ensure_index!(whole_file_buffer[optional_fuzzy_mask_start..track_data_start]);
        // Still unusued
    }

    let optional_timing_data = if optional_timing_record_size > 0 {
        let optional_timing_record =
            &ensure_index!(whole_file_buffer[track_data_end..next_track_record_offset]);

        let timing_data = read_timing_record(optional_timing_record)?;
        ensure!(timing_data.len() * 2 == timing_data_size);
        Some(timing_data)
    } else {
        None
    };

    // The optional track image is provided for emulator usage when the "Read track" command is issued
    // to the WD1772. We don't really need it as it only contains the data bits and a reconstruction
    // of flux signals is impossible with this.
    if (track_flags & TRK_IMAGE) != 0 {
        let mut track_image_header_reader =
            Cursor::new(&ensure_index!(whole_file_buffer[track_data_start..]));

        let (_first_sync_offset, track_image_start) = if (track_flags & TRK_SYNC) == 0 {
            (0, 2)
        } else {
            (
                track_image_header_reader.read_u16::<LittleEndian>()? as usize,
                4,
            )
        };

        let track_image_size = track_image_header_reader.read_u16::<LittleEndian>()? as usize;

        let _track_image_content_data =
            &ensure_index!(track_data[track_image_start..(track_image_start + track_image_size)]);

        // I had the idea that this data can be used to reconstruct a raw track from this.
        // But this is not possible because of
        // http://info-coach.fr/atari/hardware/FD-Hard.php#False_Sync_Byte_Pattern
        // It seems that Read Track is highly flawed in the WD1772 and therefore will
        // rarely deliver data that makes sense.
    }

    // If the sector count is 0, this is defined to be an empty or unformatted track.
    if sector_count == 0 {
        return Ok((None, next_track_record_offset));
    }

    ensure!(
        (track_flags & TRK_SECT) != 0,
        "Having no sector descriptors is currently not supported."
    );

    // We start writing actual track data now using the sorted sectors.

    let trackbuf: RefCell<Vec<u8>> = RefCell::new(Vec::new());
    let mut collector = BitStreamCollector::new(|f| trackbuf.borrow_mut().push(f));
    let mut encoder = MfmEncoder::new(|cell| collector.feed(cell));

    let mut deviation_map: Vec<SectorTimingDeviation> = Vec::new();
    let mut byte_position_offset = None;

    for sector in &sectors {
        // Optional patching to remove sectors.
        // This is required in case a sector is inside another.
        // Turrican requires this.
        if patch_discard_sector(sector, file_hash_str) {
            continue;
        }

        // calculate the assumed cell size for this sector
        // the read time is the time it takes to read the data section in microseconds.
        // This is slightly problematic as the gaps are not considered here.
        // if the read time is 0, the "standard read time" has to be assumed.
        let cell_size_in_seconds = if sector.read_time == 0 {
            2e-6
        } else {
            read_time_to_cellsize_in_seconds(sector.read_time as u16, sector.sector_size)
        };

        // The gap sizes are not part of the stx file. We are generating them on the fly
        // based on the bit position in the sector descriptor which can be transformed into
        // byte positions.
        let byte_position_offset_value =
            *byte_position_offset.get_or_insert(if sectors.len() == 1 {
                0
            } else {
                sector.bit_position / 4
            });

        let mfm_word_position = sector.bit_position / 4 - byte_position_offset_value;
        let dynamic_gap_size = (mfm_word_position as i32 - trackbuf.borrow().len() as i32) / 2;

        if dynamic_gap_size >= 0 {
            generate_iso_gap(dynamic_gap_size as usize, 0x4e, &mut encoder);
        }

        let custom_sector = patch_custom_sector(
            sector,
            file_hash_str,
            &mut encoder,
            &mut has_non_flux_reversal_area,
        );

        if !custom_sector {
            // No special code required to fix this sector? Then do a normal ISO one.

            let sector_data = &ensure_index!(
                track_data[sector.data_offset..(sector.data_offset + sector.sector_size)]
            );

            // sector header preamble with 0x00
            generate_iso_gap(gap2_size, 0, &mut encoder);

            encoder.feed(MfmWord::SyncWord);
            encoder.feed(MfmWord::SyncWord);
            encoder.feed(MfmWord::SyncWord);

            // usually we would have a function to generate a header. but STX is rather special
            // as this code allows wrong sector header CRCs as STX files support that.
            let sector_header = vec![
                ISO_IDAM,
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
            generate_iso_gap(gap3a_size, 0x4e, &mut encoder);

            // now the actual data of the sector
            generate_iso_data_header(gap3b_size, &mut encoder, None);

            if (sector.fdc_flags & FDC_FLAG_INTRA_SECTOR_BIT_WIDTH_VARIATION) != 0 {
                // TODO: This code was never tested.
                // I'm unable to find an image which uses only this and nothing
                // else abstract to protect itself.
                let timing_data = optional_timing_data
                    .as_ref()
                    .context(program_flow_error!())?;

                let mut crc = crc16::State::<crc16::CCITT_FALSE>::new();
                crc.update(&[ISO_SYNC_BYTE, ISO_SYNC_BYTE, ISO_SYNC_BYTE, ISO_DAM]);
                crc.update(sector_data);
                let crc16 = crc.get();

                let sector_data_chunks = sector_data.chunks_exact(16);
                ensure!(sector_data_chunks.len() == timing_data.len());

                sector_data_chunks.zip(timing_data.iter()).for_each(|f| {
                    f.0.iter().for_each(|byte| encoder.feed_encoded8(*byte));

                    let raw_bytes_to_add = trackbuf.borrow().len()
                        - total_number_raw_bytes_deviation_map(&deviation_map);

                    deviation_map.push(SectorTimingDeviation {
                        number_of_raw_bytes: raw_bytes_to_add,
                        cell_size_in_seconds: *f.1,
                    })
                });

                encoder.feed_encoded8((crc16 >> 8) as u8);
                encoder.feed_encoded8((crc16 & 0xff) as u8);
            } else if (sector.fdc_flags & (FDC_FLAG_CRC_ERROR | FDC_FLAG_RECORD_NOT_FOUND))
                == FDC_FLAG_CRC_ERROR
            {
                generate_iso_data_with_broken_crc(sector_data, &mut encoder);
            } else {
                generate_iso_data_with_crc(sector_data, &mut encoder, None);
            }
        }

        // variable density calculation.
        // how much raw bytes were added? pack these together
        // with the current density of this sector
        let raw_bytes_to_add =
            trackbuf.borrow().len() - total_number_raw_bytes_deviation_map(&deviation_map);

        deviation_map.push(SectorTimingDeviation {
            number_of_raw_bytes: raw_bytes_to_add,
            cell_size_in_seconds,
        })
    }

    // end the track
    let dynamic_gap5_size = (track_length * 2 - trackbuf.borrow().len()) / 2;
    generate_iso_gap(dynamic_gap5_size, 0x4e, &mut encoder);

    ensure!(
        track_length * 2 >= trackbuf.borrow().len(),
        "trackbuf too long!"
    );

    // fill out remaining cells after ending the track.
    let raw_bytes_to_add =
        trackbuf.borrow().len() - total_number_raw_bytes_deviation_map(&deviation_map);
    deviation_map
        .last_mut()
        .context(program_flow_error!())?
        .number_of_raw_bytes += raw_bytes_to_add;

    let densitymap = convert_timing_deviation_to_densitymap(deviation_map)?;

    ensure!(!densitymap.is_empty());

    let track = RawTrack::new_with_non_flux_reversal_area(
        u32::from(cylinder),
        u32::from(head),
        trackbuf.take(),
        densitymap,
        util::Encoding::MFM,
        has_non_flux_reversal_area,
    );

    Ok((Some(track), next_track_record_offset))
}
