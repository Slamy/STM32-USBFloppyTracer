use crate::rawtrack::auto_cell_size;
use crate::rawtrack::{RawImage, RawTrack};
use std::cell::Cell;
use std::ffi::CString;
use std::mem::{self, MaybeUninit};
use std::slice;
use std::sync::Mutex;
use util::{DensityMap, DensityMapEntry, PulseDuration, DRIVE_3_5_RPM};

// Information source:
// http://www.softpres.org/_media/files:ipfdoc102a.zip?id=download&cache=cache

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

fn sparse_timebuf(timebuf: &[u32]) -> DensityMap {
    let mut current_val = *timebuf.first().unwrap();
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
        sparse_timebuf.iter().map(|f| f.number_of_cellbytes).sum()
    );

    sparse_timebuf
}

pub fn parse_ipf_image(path: &str) -> RawImage {
    println!("Reading IPF from {path} ...");

    let mut tracks: Vec<RawTrack> = Vec::new();

    // The CAPS libary is not thread safe!
    // In unit tests this can become an issue as this code is called multiple
    // times. We need a mutex so the code between CAPSInit()
    // and CAPSExit() is not processed in multiple threads.
    static caps_mutex: Mutex<Cell<()>> = Mutex::new(Cell::new(()));
    let caps_mutex_guard = caps_mutex.lock();

    assert!(unsafe { CAPSInit() == 0 });

    let id = unsafe { CAPSAddImage() };

    let mut cii = MaybeUninit::<CapsImageInfo>::uninit();
    let cpath = CString::new(path).unwrap().into_raw();
    assert!(unsafe { CAPSLockImage(id, cpath) == 0 });
    let _ = unsafe { CString::from_raw(cpath) };

    assert!(unsafe { CAPSGetImageInfo(cii.as_mut_ptr(), id) == 0 });

    let cii = unsafe { cii.assume_init_mut() };

    for cylinder in cii.mincylinder..=cii.maxcylinder {
        for head in cii.minhead..=cii.maxhead {
            let mut trackInf = MaybeUninit::<CapsTrackInfoT1>::uninit();

            assert_eq!(
                unsafe {
                    (*trackInf.as_mut_ptr()).type_ = 1;
                    CAPSLockTrack(
                        trackInf.as_mut_ptr().cast::<std::ffi::c_void>(),
                        id,
                        cylinder,
                        head,
                        FLAG_LOCK_TYPE | FLAG_LOCK_INDEX | FLAG_LOCK_DENVAR,
                    )
                },
                0
            );

            let trackInf = unsafe { trackInf.assume_init_mut() };

            if trackInf.tracklen > 0 {
                // Some tracks have more than one rotation inside. The overlap must be removed
                // as that additional data would increase writing frequency.
                // It is also possible that the overlap position contains
                // invalid MFM data...
                let overlap = trackInf.overlap;

                let trackbuf_orig =
                    unsafe { slice::from_raw_parts(trackInf.trackbuf, trackInf.tracklen as usize) };

                let trackbuf: Vec<u8> = if overlap == -1 {
                    // No overlap
                    trackbuf_orig.into()
                } else if overlap < 10 {
                    // Some images have the overlap at the beginning
                    trackbuf_orig[1 + overlap as usize..].into()
                } else {
                    // We have some overlap at the end
                    assert!(
                        trackInf.tracklen >= overlap as u32,
                        "Overlap behind end of data?"
                    );
                    trackbuf_orig[0..overlap as usize].into()
                };

                let auto_cell_size =
                    auto_cell_size(trackbuf.len() as u32, DRIVE_3_5_RPM).min(168.0_f64);

                let mut densitymap;
                if trackInf.type_ == ctitVar as u32 {
                    println!(
                        "Variable Density Track {cylinder} {head} - Auto cell size {auto_cell_size} "
                    );

                    assert!(trackInf.timelen == trackInf.tracklen);

                    let timebuf_orig = unsafe {
                        slice::from_raw_parts(trackInf.timebuf, trackInf.timelen as usize).to_vec()
                    };

                    let timebuf: Vec<u32> = if overlap == -1 {
                        // No overlap
                        timebuf_orig
                    } else if overlap < 10 {
                        // Some images have the overlap at the beginning
                        timebuf_orig[1 + overlap as usize..].into()
                    } else {
                        // We have some overlap at the end
                        assert!(
                            trackInf.timelen >= overlap as u32,
                            "Overlap behind end of data?"
                        );
                        timebuf_orig[0..overlap as usize].into()
                    };

                    densitymap = sparse_timebuf(&timebuf);

                    for d in &mut densitymap {
                        d.cell_size = PulseDuration(
                            (f64::from(d.cell_size.0) * auto_cell_size / 1000.0) as i32,
                        );
                    }
                } else {
                    densitymap = vec![DensityMapEntry {
                        number_of_cellbytes: trackbuf.len(),
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
        CAPSExit();
    }

    // It is now safe to drop the guard as we have finished using the CAPS library
    mem::drop(caps_mutex_guard);

    let smallest_cell_size = tracks
        .iter()
        .map(|f| {
            f.densitymap
                .iter()
                .map(|f| f.cell_size.0)
                .reduce(std::cmp::Ord::min)
        })
        .map(std::option::Option::unwrap)
        .reduce(std::cmp::Ord::min)
        .unwrap();
    let smallest_cell_size_usec = f64::from(smallest_cell_size) / 84.0;
    println!(
        "Smallest cell size of this image is {smallest_cell_size} / {smallest_cell_size_usec:.2} usec"
    );

    RawImage {
        tracks,
        disk_type: util::DiskType::Inch3_5,
        density: util::Density::SingleDouble,
    }
}
