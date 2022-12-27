use crate::md5_sum_of_file;
use crate::rawtrack::auto_cell_size;
use crate::rawtrack::{RawImage, RawTrack};
use std::ffi::{c_void, CString};
use std::mem::MaybeUninit;
use std::slice;
use util::{DensityMap, DensityMapEntry, PulseDuration, DRIVE_3_5_RPM};

// Information source:
// http://www.softpres.org/_media/files:ipfdoc102a.zip?id=download&cache=cache

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

fn sparse_timebuf(timebuf: &Vec<u32>) -> DensityMap {
    let mut current_val = *timebuf.get(0).unwrap();
    let mut density_active_for: u32 = 0;

    let mut sparse_timebuf = Vec::new();

    for density in timebuf.iter() {
        density_active_for += 1;

        if current_val != *density {
            sparse_timebuf.push(DensityMapEntry {
                number_of_cellbytes: density_active_for as usize,
                cell_size: PulseDuration(current_val as i32),
            });
            current_val = *density;
            density_active_for = 0;
        }
    }

    if density_active_for > 0 {
        sparse_timebuf.push(DensityMapEntry {
            number_of_cellbytes: density_active_for as usize,
            cell_size: PulseDuration(current_val as i32),
        });
    }

    // ensure that the lengths do match up!
    assert_eq!(
        timebuf.len(),
        sparse_timebuf
            .iter()
            .map(|f| (*f).number_of_cellbytes)
            .sum()
    );

    sparse_timebuf
}

fn patch_trackdata(source: &[u8], file_hash_str: &str, cyl: u32, head: u32) -> Vec<u8> {
    match (file_hash_str, cyl, head) {
        // Enchanted land has a broken cell at the end of the last track
        ("d907e262b6a3a72e0c690216bb9d0290", 79, 0) => {
            let mut edit: Vec<u8> = source.into();
            edit[12606] = 0x55;
            edit
        }

        // Gods Disk 1 has invalid Mfm Encoding on variable densitiy track
        ("7b2a11eda49fc6841834e792dab53997", 0, 1) => {
            let mut edit: Vec<u8> = source.into();
            edit[0] = 0x55;
            edit
        }

        _ => source.into(),
    }
}

pub fn parse_ipf_image(path: &str) -> RawImage {
    println!("Reading IPF from {} ...", path);

    let file_hashstr = md5_sum_of_file(path);
    let mut tracks: Vec<RawTrack> = Vec::new();

    assert!(unsafe { CAPSInit() == 0 });

    let id = unsafe { CAPSAddImage() };

    let mut cii = MaybeUninit::<CapsImageInfo>::uninit();
    let cpath = CString::new(path).unwrap().into_raw();
    assert!(unsafe { CAPSLockImage(id, cpath) == 0 });
    let _ = unsafe { CString::from_raw(cpath) };

    assert!(unsafe { CAPSGetImageInfo(cii.as_mut_ptr(), id) == 0 });

    let cii = unsafe { cii.assume_init_mut() };

    for cylinder in cii.mincylinder..cii.maxcylinder + 1 {
        for head in cii.minhead..cii.maxhead + 1 {
            let mut trackInf = MaybeUninit::<CapsTrackInfo>::uninit();

            assert!(unsafe {
                CAPSLockTrack(
                    trackInf.as_mut_ptr() as *mut c_void,
                    id,
                    cylinder,
                    head,
                    DI_LOCK_INDEX | DI_LOCK_DENVAR,
                ) == 0
            });

            let trackInf = unsafe { trackInf.assume_init_mut() };

            if trackInf.tracklen > 0 {
                let auto_cell_size = auto_cell_size(trackInf.tracklen, DRIVE_3_5_RPM);

                let trackbuf_orig =
                    unsafe { slice::from_raw_parts(trackInf.trackbuf, trackInf.tracklen as usize) };

                let trackbuf = patch_trackdata(trackbuf_orig, &file_hashstr, cylinder, head);

                let mut densitymap;
                if trackInf.type_ == ctitVar {
                    println!(
                        "Variable Density Track {} {} - Auto cell size {} ",
                        cylinder, head, auto_cell_size
                    );
                    let timebuf = unsafe {
                        slice::from_raw_parts(trackInf.timebuf, trackInf.timelen as usize).to_vec()
                    };

                    densitymap = sparse_timebuf(&timebuf);

                    densitymap.iter_mut().for_each(|d| {
                        d.cell_size = PulseDuration(
                            ((d.cell_size.0 as f64) * auto_cell_size / 1000.0) as i32,
                        );
                    });
                } else {
                    densitymap = vec![DensityMapEntry {
                        number_of_cellbytes: trackbuf.len() as usize,
                        cell_size: PulseDuration(auto_cell_size as i32),
                    }];
                }

                tracks.push(RawTrack::new(
                    cylinder,
                    head,
                    trackbuf,
                    densitymap,
                    util::Encoding::MFM,
                ));
            }
            unsafe {
                CAPSUnlockTrack(id, cylinder, head);
            }
        }
    }
    unsafe {
        CAPSUnlockImage(id);
        CAPSRemImage(id);

        // Usually we would free library memory here using CAPSExit();
        // But the problem is that libcaps can't be used after doing so.
        // especially for unit tests, this is a problem.
        // CAPSExit();
    }

    let smallest_cell_size = tracks
        .iter()
        .map(|f| {
            f.densitymap
                .iter()
                .map(|f| f.cell_size.0)
                .reduce(|a, b| a.min(b))
        })
        .map(|f| f.unwrap())
        .reduce(|a, b| a.min(b))
        .unwrap();
    let smallest_cell_size_usec = smallest_cell_size as f64 / 84.0;
    println!(
        "Smallest cell size of this image is {} / {:.2} usec",
        smallest_cell_size, smallest_cell_size_usec
    );

    RawImage {
        tracks,
        disk_type: util::DiskType::Inch3_5,
        density: util::Density::SingleDouble,
    }
}
