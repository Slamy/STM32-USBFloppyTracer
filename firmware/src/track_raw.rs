use core::{cell::RefCell, future::Future, task::Poll};

use alloc::collections::VecDeque;
use cassette::futures::poll_fn;
use heapless::spsc::{Consumer, Producer};

use util::{
    bitstream::to_bit_stream, fluxpulse::FluxPulseGenerator2, PulseDuration, RawCellData, Track,
};

use crate::{
    green,
    interrupts::{
        self, async_select_and_wait_for_track, async_wait_for_index, FLUX_READER, FLUX_WRITER,
    },
    safeiprintln,
};

pub struct RawTrackWriter {
    pub read_cons: Consumer<'static, u32, 512>,
    pub write_prod_cell: RefCell<Producer<'static, u32, 128>>,
    pub track_data_to_write: Option<RawCellData>,
}

impl RawTrackWriter {
    fn async_read_flux(&mut self) -> impl Future<Output = u32> + '_ {
        poll_fn(move |_| {
            if let Some(x) = self.read_cons.dequeue() {
                Poll::Ready(x)
            } else {
                Poll::Pending
            }
        })
    }

    pub async fn write_and_verify(
        &mut self,
        track: Track,
        first_significance_offset: usize,
    ) -> (u8, u8, bool) {
        async_select_and_wait_for_track(track).await;

        let mut write_operations = 0;
        let mut verify_operations = 0;

        for _ in 0..5 {
            safeiprintln!(
                "Write track at cyl:{} head:{} sigoff:{}",
                track.cylinder.0,
                track.head.0,
                first_significance_offset
            );
            write_operations += 1;
            self.write_track().await;

            for _ in 0..3 {
                verify_operations += 1;
                if self.verify_track(first_significance_offset).await {
                    return (write_operations, verify_operations, true);
                }
            }
        }
        return (write_operations, verify_operations, false);
    }

    async fn write_track(&mut self) {
        // keep it spinning!
        cortex_m::interrupt::free(|cs| {
            interrupts::FLOPPY_CONTROL
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .spin_motor();
        });

        // prefill output buffer
        let track_data_to_write = self.track_data_to_write.take().unwrap();
        let mut parts = track_data_to_write.borrow_parts().iter();
        let part = parts.next().unwrap();

        let mut write_prod_fpg = FluxPulseGenerator2::new(
            |f| {
                self.write_prod_cell
                    .borrow_mut()
                    .enqueue(f.0 as u32)
                    .unwrap()
            },
            part.cell_size.0 as u32,
        );
        let mut track_data_iter = part.cells.iter();
        for _ in 0..8 {
            let mfm_byte = *track_data_iter.next().unwrap();
            to_bit_stream(mfm_byte, |bit| write_prod_fpg.feed(bit));
        }

        // start transmit on index pulse
        async_wait_for_index().await;

        cortex_m::interrupt::free(|cs| {
            let mut fr1 = FLUX_WRITER.borrow(cs).borrow_mut();
            let y2 = fr1.as_mut().unwrap();
            y2.start_transmit(cs);
        });

        // continue until whole track is written.
        // TODO copy pasta
        while let Some(mfm_byte) = track_data_iter.next() {
            while self.write_prod_cell.borrow().len() > 70 {
                cassette::yield_now().await;
            }
            to_bit_stream(*mfm_byte, |bit| write_prod_fpg.feed(bit));
        }

        while let Some(part) = parts.next() {
            let mut track_data_iter = part.cells.iter();

            write_prod_fpg.cell_duration = part.cell_size.0 as u32;
            while let Some(mfm_byte) = track_data_iter.next() {
                while self.write_prod_cell.borrow().len() > 70 {
                    cassette::yield_now().await;
                }
                to_bit_stream(*mfm_byte, |bit| write_prod_fpg.feed(bit));
            }
        }

        self.track_data_to_write = Some(track_data_to_write);
    }

    async fn verify_track(&mut self, first_significance_offset: usize) -> bool {
        cortex_m::interrupt::free(|cs| {
            interrupts::FLOPPY_CONTROL
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .spin_motor();
        });

        while let Some(_) = self.read_cons.dequeue() { // Throw away
        }

        let track_data_to_write = self.track_data_to_write.take().unwrap();

        let mut parts = track_data_to_write.borrow_parts().iter();
        let part = parts.next().unwrap();

        //prepare compare data
        let flux_data_to_write_queue: RefCell<VecDeque<PulseDuration>> =
            RefCell::new(VecDeque::with_capacity(first_significance_offset + 100));
        let mut flux_data_to_write_fpg = FluxPulseGenerator2::new(
            |f| flux_data_to_write_queue.borrow_mut().push_back(f),
            part.cell_size.0 as u32,
        );
        let mut track_data_to_write_iter = part.cells.iter();
        while flux_data_to_write_queue.borrow().len() < first_significance_offset + 12 {
            to_bit_stream(
                *track_data_to_write_iter.next().unwrap_or_else(|| {
                    panic!("Not filled {}", flux_data_to_write_queue.borrow().len())
                }),
                |bit| flux_data_to_write_fpg.feed(bit),
            )
        }

        if first_significance_offset > 6 {
            flux_data_to_write_queue
                .borrow_mut()
                .drain(0..first_significance_offset - 6);
        } else {
            // TODO For Apydia
            flux_data_to_write_queue.borrow_mut().drain(0..2);
        }

        let mut read_mfm_flux_data_queue: VecDeque<PulseDuration> = VecDeque::with_capacity(1000);

        // start reception of track on next index pulse
        async_wait_for_index().await;

        cortex_m::interrupt::free(|cs| {
            let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
            let y2 = fr1.as_mut().unwrap();
            y2.start_reception(cs);
        });

        // throw away first pulses before the point of significance
        if first_significance_offset > 10 {
            let mut pulses_to_throw_away = first_significance_offset - 10;
            while pulses_to_throw_away > 0 {
                let _ = self.async_read_flux().await;
                pulses_to_throw_away -= 1;
            }
        }

        // now record something slightly larger than the "significant window"
        let read_data_window_size = 30;

        while read_mfm_flux_data_queue.len() < read_data_window_size {
            let x = self.async_read_flux().await;
            read_mfm_flux_data_queue.push_back(PulseDuration(x as u16))
        }

        let mut equal = false;
        let significance_window_size = 12;
        let mut offset: i32 = -1;

        // now move the reference significant window over the already read data and compare it.
        // there should be one position where it matches!
        for i in 0..read_data_window_size {
            if read_mfm_flux_data_queue.len() < significance_window_size {
                safeiprintln!("No data sync!");

                cortex_m::interrupt::free(|cs| {
                    let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
                    let y2 = fr1.as_mut().unwrap();
                    y2.stop_reception(cs);
                });
                self.track_data_to_write = Some(track_data_to_write);

                return false;
            }
            equal = read_mfm_flux_data_queue
                .range(0..significance_window_size)
                .zip(flux_data_to_write_queue.borrow().iter())
                .all(|(x, y)| y.similar(x));

            if equal {
                offset = i as i32;
                break;
            }

            read_mfm_flux_data_queue.pop_front();
        }

        assert!(equal);
        // We are now synchronized.
        assert!(offset >= 0);

        let mut successful_compares = 0;
        let mut accumulated_diff = 0;
        loop {
            if read_mfm_flux_data_queue.len() < 30 {
                let x = self.async_read_flux().await;
                read_mfm_flux_data_queue.push_back(PulseDuration(x as u16))
            }

            if flux_data_to_write_queue.borrow().len() < 30 {
                if let Some(val) = track_data_to_write_iter.next() {
                    to_bit_stream(*val, |bit| flux_data_to_write_fpg.feed(bit))
                } else {
                    if let Some(part) = parts.next() {
                        flux_data_to_write_fpg.cell_duration = part.cell_size.0 as u32;

                        track_data_to_write_iter = part.cells.iter();
                    }
                }
            }

            if flux_data_to_write_queue.borrow().is_empty() {
                break;
            }

            if !read_mfm_flux_data_queue.is_empty() && !flux_data_to_write_queue.borrow().is_empty()
            {
                let reference = flux_data_to_write_queue.borrow_mut().pop_front().unwrap();
                let readback = read_mfm_flux_data_queue.pop_front().unwrap();

                accumulated_diff += (reference.0 as i32).abs_diff(readback.0 as i32) / 8;
                if !reference.similar(&readback) {
                    safeiprintln!(
                        "{} != {}, successful_compares until compare fail: {}",
                        reference.0,
                        readback.0,
                        successful_compares
                    );

                    cortex_m::interrupt::free(|cs| {
                        let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
                        let y2 = fr1.as_mut().unwrap();
                        y2.stop_reception(cs);
                    });
                    self.track_data_to_write = Some(track_data_to_write);

                    return false;
                }
                successful_compares += 1;
            }
        }

        cortex_m::interrupt::free(|cs| {
            let mut fr1 = FLUX_READER.borrow(cs).borrow_mut();
            let y2 = fr1.as_mut().unwrap();
            y2.stop_reception(cs);
        });

        safeiprintln!(
            "We did it! We have verified {} pulses. But {}",
            successful_compares,
            accumulated_diff
        );
        true
    }
}
