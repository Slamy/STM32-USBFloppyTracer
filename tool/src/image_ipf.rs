use std::ffi::{c_void, CString};
use std::mem::MaybeUninit;

use std::slice;

use util::{DensityMapEntry, PulseDuration};

use crate::rawtrack::RawTrack;

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

fn sparse_timebuf(timebuf: &Vec<u32>) -> Vec<DensityMapEntry> {
    let mut current_val = *timebuf.get(0).unwrap();
    let mut density_active_for: u32 = 0;

    let mut sparse_timebuf = Vec::new();

    for density in timebuf.iter() {
        density_active_for += 1;

        if current_val != *density {
            sparse_timebuf.push(DensityMapEntry {
                number_of_cells: density_active_for as usize,
                cell_size: PulseDuration(current_val as u16),
            });
            current_val = *density;
            density_active_for = 0;
        }
    }

    if density_active_for > 0 {
        sparse_timebuf.push(DensityMapEntry {
            number_of_cells: density_active_for as usize,
            cell_size: PulseDuration(current_val as u16),
        });
    }

    // ensure that the lengths do match up!
    assert_eq!(
        timebuf.len(),
        sparse_timebuf.iter().map(|f| (*f).number_of_cells).sum()
    );

    sparse_timebuf
}

fn auto_cell_size(tracklen: u32) -> f64 {
    let number_cells = tracklen * 8;
    let rpm = 301.0; // Normally 300 RPM would be correct. But the drive might be faster. Let's be safe here.
    let seconds_per_revolution = 60.0 / rpm;
    let microseconds_per_cell = 10_f64.powi(6) * seconds_per_revolution / number_cells as f64;
    let stm_timer_mhz = 84.0;
    let raw_timer_val = stm_timer_mhz * microseconds_per_cell;
    raw_timer_val
}

pub fn parse_ipf_image(path: &str) -> Vec<RawTrack> {
    println!("Reading IPF from {} ...", path);

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
                let auto_cell_size = auto_cell_size(trackInf.tracklen);

                let trackbuf = unsafe {
                    slice::from_raw_parts(trackInf.trackbuf, trackInf.tracklen as usize).to_vec()
                };

                let mut densitymap;
                if trackInf.type_ == ctitVar {
                    let timebuf = unsafe {
                        slice::from_raw_parts(trackInf.timebuf, trackInf.timelen as usize).to_vec()
                    };

                    densitymap = sparse_timebuf(&timebuf);

                    densitymap.iter_mut().for_each(|d| {
                        d.cell_size = PulseDuration(
                            ((d.cell_size.0 as f64) * auto_cell_size / 1000.0) as u16,
                        );
                    });
                } else {
                    densitymap = vec![DensityMapEntry {
                        number_of_cells: trackbuf.len() as usize,
                        cell_size: PulseDuration(auto_cell_size as u16),
                    }];
                }

                tracks.push(RawTrack::new(cylinder, head, trackbuf, densitymap));
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

    tracks
}
