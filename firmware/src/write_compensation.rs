use crate::{safeiprintln, step_control::FloppyControl};
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use heapless::spsc::{Consumer, Producer, Queue};
use stm32f4xx_hal::pac::{interrupt, Interrupt};
use stm32f4xx_hal::{pac, prelude::*};
use util::fluxpulse::{FluxPulseGenerator2, FluxPulseToCells2};

use util::{mfm::*, PulseDuration};

use crate::{orange, FLUX_WRITER};
use crate::{FLUX_READER, INDEX_OCCURED};

fn calibrate_write_compensation(
    mut write_prod: Producer<u32, 128>,
    mut read_cons: Consumer<u32, 128>,
    mut syst: pac::SYST,
) {
    for _ in 0..2 {
        while INDEX_OCCURED.swap(false, Ordering::Relaxed) == false {}
        orange(true);
        safeiprintln!("Index");
        orange(false);
    }

    let mut fpg = FluxPulseGenerator2::new(|f| write_prod.enqueue(f.0 as u32).unwrap(), 168);
    let mut mfme = MfmEncoder2::new(|f| fpg.feed(f));

    //let compensation = 0;
    //let compensation = 6;
    mfme.feed(MfmResult2::Got(0));
    mfme.feed(MfmResult2::Got(0));
    mfme.feed(MfmResult2::Got(0));
    mfme.feed(MfmResult2::Got(0));
    mfme.feed(MfmResult2::SyncWord);
    mfme.feed(MfmResult2::SyncWord);
    mfme.feed(MfmResult2::SyncWord);
    mfme.feed(MfmResult2::Got(0));

    let raw = vec![
        252, 252, 252, 168, 168, 168, 168, 252, 252, 252, 252, 168, 252, 252,
    ];

    let write = vec![
        252,
        252,
        252 + 6,
        168 - 6,
        168,
        168,
        168 - 6,
        252 + 6,
        252,
        252,
        252 + 8,
        168 - 16,
        252 + 8,
        252,
    ];

    for i in write.iter() {
        write_prod.enqueue(*i).unwrap();
    }

    while INDEX_OCCURED.swap(false, Ordering::Relaxed) == false {}

    cortex_m::interrupt::free(|cs| {
        let mut fr1 = FLUX_WRITER.borrow(cs).borrow_mut();
        let y2 = fr1.as_mut().unwrap();
        y2.start_transmit(cs);
    });

    {
        let mut finished = false;
        while !finished {
            cortex_m::interrupt::free(|cs| {
                let mut fr1 = FLUX_WRITER.borrow(cs).borrow_mut();
                let y2 = fr1.as_mut().unwrap();
                if !y2.transmission_active() {
                    finished = true;
                }
            });
        }
    }

    for _ in 0..2 {
        while INDEX_OCCURED.swap(false, Ordering::Relaxed) == false {}

        cortex_m::interrupt::free(|cs| {
            let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
            let y2 = fr1.as_mut().unwrap();
            y2.start_reception(cs);
        });

        let finished = AtomicBool::new(false);
        let insync = AtomicBool::new(false);

        let mut read_collect = Vec::with_capacity(100);
        let mut read_collect2 = Vec::with_capacity(100);

        let mut mfmd = MfmDecoder2::new(|f| {
            if matches!(f, MfmResult2::SyncWord) {
                insync.store(true, Ordering::Relaxed);
            }

            if let MfmResult2::Got(x) = f {
                read_collect.push(x);

                if read_collect.len() >= 4 {
                    finished.store(true, Ordering::Relaxed);
                }
            }
        });
        let mut fptc = FluxPulseToCells2::new(|f| mfmd.feed(f), 84);

        while !finished.load(Ordering::Relaxed) {
            while let Some(duration) = read_cons.dequeue() {
                if insync.load(Ordering::Relaxed) {
                    read_collect2.push(duration);

                    if read_collect2.len() > 50 {
                        finished.store(true, Ordering::Relaxed);
                    }
                }
                fptc.feed(PulseDuration(duration as u16));
            }
        }
        cortex_m::interrupt::free(|cs| {
            let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
            let y2 = fr1.as_mut().unwrap();
            y2.stop_reception(cs);
        });

        let read_back = &read_collect2[7..7 + raw.len()];
        safeiprintln!("rba {:?}", read_back);
        safeiprintln!("raw {:?}", raw);

        let compare: Vec<i32> = read_back
            .iter()
            .zip(&raw)
            .map(|f| *f.0 as i32 - *f.1 as i32)
            .collect();

        //safeiprintln!("Feddig {:?}", read_collect);
        //safeiprintln!("Feddig2 {:?}", read_collect2);
        safeiprintln!("compare {:?}", compare);
    }
}
