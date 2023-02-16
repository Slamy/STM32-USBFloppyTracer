use std::{ffi::OsStr, path::Path};

use crate::rawtrack::RawImage;

use self::{
    image_adf::parse_adf_image, image_d64::parse_d64_image, image_dsk::parse_dsk_image,
    image_g64::parse_g64_image, image_ipf::parse_ipf_image, image_iso::parse_iso_image,
    image_stx::parse_stx_image,
};

pub mod image_adf;
pub mod image_d64;
pub mod image_dsk;
pub mod image_g64;
pub mod image_ipf;
pub mod image_iso;
pub mod image_stx;

pub fn parse_image(path: &str) -> RawImage {
    let extension = Path::new(path)
        .extension()
        .and_then(OsStr::to_str)
        .expect("Unknown file extension!");

    match extension {
        "ipf" => parse_ipf_image(path),
        "adf" => parse_adf_image(path),
        "d64" => parse_d64_image(path),
        "g64" => parse_g64_image(path),
        "st" => parse_iso_image(path),
        "img" => parse_iso_image(path),
        "stx" => parse_stx_image(path),
        "dsk" => parse_dsk_image(path),
        _ => panic!("{} is an unknown file extension!", extension),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File},
        io::Read,
    };

    use super::*;
    use rstest::rstest;
    use util::{DRIVE_3_5_RPM, DRIVE_5_25_RPM};

    fn md5_sum_of_file(path: &str) -> String {
        let mut f = File::open(path).expect("no file found");
        let metadata = fs::metadata(path).expect("unable to read metadata");

        let mut whole_file_buffer: Vec<u8> = vec![0; metadata.len() as usize];
        let bytes_read = f.read(whole_file_buffer.as_mut()).unwrap();
        assert_eq!(bytes_read, metadata.len() as usize);
        let file_hash = md5::compute(&whole_file_buffer);
        let file_hashstr = format!("{file_hash:x}");
        file_hashstr
    }

    #[rstest]
    #[case( // 1 - Standard ADF
        "../images/turrican.adf",
        "6677ce6cea38dc66be40e9211576a149",
        "b9167a41464460a0b4ebd8ddccd38f74"
    )]
    #[case( // 2 - Long Tracks Amiga
        "../images/Turrican.ipf",
        "654e52bec1555ab3802c21f6ea269e64",
        "428c9e69290efae8282f300f2a9ecea4"
    )]
    #[case( // 3 - Long Tracks Amiga
        "../images/Turrican2.ipf",
        "17abf9d8d5b2af451897f6db8c7f4868",
        "623564a1f6b1ec2dd1998cca3fd637af"
    )]
    #[case( // 4 - Standard D64
        "../images/Katakis_(CPX).d64",
        "a1a64b89c44d9c778b2677b0027e015e",
        "5841acd589f5208ed16219a7f080f9d7"
    )]
    #[case( // 5 - Buggy G64
        "../images/Katakis (Side 1).g64",
        "53c47c575d057181a1911e6653229324",
        "f0d02066cb590698bcf5b34573df61f7"
    )]
    #[case( // 6 - Custom STX
        "../images/Turrican (1990)(Rainbow Arts).stx",
        "4865957cd83562547a722c95e9a5421a",
        "8367a02c247e80d230f01c1841dddf1b"
    )]
    #[case( // 7 - Custom STX
        "../images/Turrican II (1991)(Rainbow Arts).stx",
        "fb96a28ad633208a973e725ceb67c155",
        "e142a9326a16ffb1c13aeaabb2856b20"
    )]
    #[case( // 8 - STX with CopyLock
        "../images/rodland.stx",
        "80f6322934ca1c76bb04b5c4d6d25097",
        "9dab1e0732200311eff31feb77bc1a87"
    )]
    #[case( // 9 - Amiga IPF with CopyLock
        "../images/Gods_Disc1.ipf",
        "7b2a11eda49fc6841834e792dab53997",
        "667d9427117a0315a43af130de921aff"
    )]
    #[case( // 10 - Atari ST IPF with LongTracks
        "../atarist_ipf/Turrican II - The Final Fight (Europe) (Budget - Kixx).ipf",
        "f18557040f7370b5c682456e668412ef",
        "912190575610bf1834dff1c80d629f87"
    )]
    #[case( // 11 - Atari ST Raw ISO Image
        "../images/Rodland (1991)(Sales Curve)[cr Alien][t].st",
        "a1ee8d4fdcf05b562d990267052965c2",
        "63ae9182461c8a9e34c202cbf4332e00"
    )]
    #[case( // 12 - Amstrad CPC 2 sided DSK Image
        "../images/R-Type_128K_dualside.dsk",
        "8bd150d9c57dc0a016db759e8dc903e2",
        "022d98d018f1aa871a0239c260ad4e11"
    )]
    fn known_image_regression_test(
        #[case] filepath: &str,
        #[case] expected_file_md5: &str,
        #[case] expected_md5: &str,
    ) {
        // before we start, we must be sure that this is really the file we want to process
        assert_eq!(
            md5_sum_of_file(filepath),
            expected_file_md5,
            "MD5 Sum of file not as expected."
        );

        let mut image = parse_image(filepath);

        let mut context = md5::Context::new();

        for track in &mut image.tracks {
            let rpm = match image.disk_type {
                util::DiskType::Inch3_5 => DRIVE_3_5_RPM,
                util::DiskType::Inch5_25 => DRIVE_5_25_RPM,
            };

            track.assert_fits_into_rotation(rpm);
            track.check_writability();

            context.consume(u32::to_le_bytes(track.cylinder));
            context.consume(u32::to_le_bytes(track.head));
            track.densitymap.iter().for_each(|g| {
                context.consume(i32::to_le_bytes(g.cell_size.0));
                context.consume(usize::to_le_bytes(g.number_of_cellbytes));
            });
            context.consume(&track.raw_data);
        }

        let md5_hash = context.compute();
        let md5_hashstr = format!("{md5_hash:x}");
        assert_eq!(md5_hashstr, expected_md5);
    }
}
