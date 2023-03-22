use core::{cell::RefCell, cmp::max, future::Future, mem, task::Poll};

use alloc::collections::VecDeque;
use cassette::futures::poll_fn;
use heapless::spsc::{Consumer, Producer};

use util::{
    bitstream::to_bit_stream, fluxpulse::FluxPulseGenerator, Bit, PulseDuration, RawCellData, Track,
};

use crate::{
    interrupts::{
        self, async_select_and_wait_for_track, async_wait_for_receive, async_wait_for_transmit,
        flux_reader_stop_reception, START_RECEIVE_ON_INDEX, START_TRANSMIT_ON_INDEX,
    },
    rprintln,
};

pub struct RawTrackHandler {
    pub read_cons: Consumer<'static, u32, 512>,
    pub write_prod_cell: RefCell<Producer<'static, u32, 128>>,
}

#[derive(Debug)]
pub enum RawTrackError {
    NoIndexPulse,
    NoIncomingData,
    NoCrossCorrelation,
    DataNotEqual,
    WriteProtected,
}

pub struct WriteVerifyError {
    pub error: RawTrackError,
    pub write_operations: u8,
    pub verify_operations: u8,
}

pub struct WriteVerifySuccess {
    pub write_operations: u8,
    pub verify_operations: u8,
    pub write_precompensation: PulseDuration,
    pub max_err: PulseDuration,
}

impl RawTrackHandler {
    fn async_read_flux(&mut self) -> impl Future<Output = Option<i32>> + '_ {
        poll_fn(move |_| {
            if let Some(pulse_duration) = self.read_cons.dequeue() {
                Poll::Ready(Some(pulse_duration as i32))
            } else {
                let motor_is_spinning = cortex_m::interrupt::free(|cs| {
                    interrupts::FLOPPY_CONTROL
                        .borrow(cs)
                        .borrow_mut()
                        .as_mut()
                        .unwrap()
                        .is_spinning()
                });

                if motor_is_spinning {
                    Poll::Pending
                } else {
                    rprintln!("async_read_flux timeout!");
                    Poll::Ready(None)
                }
            }
        })
    }

    pub async fn write_and_verify(
        &mut self,
        track: Track,
        write_precompensation: PulseDuration,
        mut raw_cell_data: RawCellData,
    ) -> Result<WriteVerifySuccess, WriteVerifyError> {
        async_select_and_wait_for_track(track).await;

        let write_protected = cortex_m::interrupt::free(|cs| {
            interrupts::FLOPPY_CONTROL
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .write_protection_is_active()
        });

        let mut write_operations = 0;
        let mut verify_operations = 0;

        if write_protected {
            rprintln!("Write Protected!");
            return Err(WriteVerifyError {
                write_operations,
                verify_operations,
                error: RawTrackError::WriteProtected,
            });
        }

        for _ in 0..5 {
            rprintln!(
                "Write track at cyl:{} head:{}",
                track.cylinder.0,
                track.head.0,
            );
            write_operations += 1;

            raw_cell_data = self
                .write_track(write_precompensation, raw_cell_data)
                .await
                .map_err(|error| WriteVerifyError {
                    error,
                    write_operations,
                    verify_operations,
                })?;

            for read_try in 0..3 {
                verify_operations += 1;

                let verify_result = self.verify_track(raw_cell_data).await;

                match verify_result {
                    Ok(max_err) => {
                        return Ok(WriteVerifySuccess {
                            write_operations,
                            verify_operations,
                            write_precompensation,
                            max_err,
                        });
                    }
                    Err((RawTrackError::DataNotEqual, track)) => {
                        // We shall do nothing. Maybe it was a fluke?
                        // Just read again...
                        raw_cell_data = track;
                    }
                    Err((RawTrackError::NoCrossCorrelation, track)) if read_try == 0 => {
                        // This happens sometimes. Nothing to worry about.
                        // This usually occurs with longer tracks as the read head
                        // must recalibrate.
                        // Just read again...
                        raw_cell_data = track;
                    }
                    Err((RawTrackError::NoCrossCorrelation, track)) => {
                        // Ok now this is bad.
                        // Abort reading and write again. This won't get any better.
                        // This can occur if the write process overwrites the start of the track.
                        // A fluctuation in the rotation speed causes this.
                        raw_cell_data = track;
                        break;
                    }
                    Err((RawTrackError::NoIncomingData, _track)) => {
                        // Abort. Drive not responding
                        return Err(WriteVerifyError {
                            write_operations,
                            verify_operations,
                            error: RawTrackError::NoIncomingData,
                        });
                    }
                    Err((RawTrackError::NoIndexPulse, _track)) => {
                        // Abort. Drive not responding
                        return Err(WriteVerifyError {
                            write_operations,
                            verify_operations,
                            error: RawTrackError::NoIndexPulse,
                        });
                    }
                    Err((RawTrackError::WriteProtected, _)) => {
                        panic!("Program flow error")
                    }
                }
            }
        }
        Err(WriteVerifyError {
            write_operations,
            verify_operations,
            error: RawTrackError::DataNotEqual,
        })
    }

    async fn feed_mfm_raw_iterator_to_writer<T>(
        &self,
        track_data_iter: core::slice::Iter<'_, u8>,
        write_prod_fpg: &mut FluxPulseGenerator<T>,
    ) where
        T: FnMut(PulseDuration),
    {
        for mfm_byte in track_data_iter {
            assert!(self.write_prod_cell.borrow().len() > 20); // check for underflow

            while self.write_prod_cell.borrow().len() > 70 {
                cassette::yield_now().await;
            }
            to_bit_stream(*mfm_byte, |bit| write_prod_fpg.feed(bit));
        }
    }

    async fn write_track(
        &mut self,
        write_precompensation: PulseDuration,
        track_data_to_write: RawCellData,
    ) -> Result<RawCellData, RawTrackError> {
        // keep it spinning!
        cortex_m::interrupt::free(|cs| {
            interrupts::FLUX_WRITER
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .clear_buffers();

            // Start degaussing the track.
            // Avoids having old data at the end of the track
            // which might cause confusion during reading without
            // index alignment. Amiga and C64 are prone to this problem
            // as they just ignore the index signal during reading and writing.
            interrupts::FLUX_WRITER
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .enable_write_head();

            interrupts::FLOPPY_CONTROL
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .spin_motor();
        });

        // prefill output buffer
        let mut parts = track_data_to_write.borrow_parts().iter();
        let part = parts.next().unwrap();

        let mut write_prod_fpg = FluxPulseGenerator::new(
            |f| {
                self.write_prod_cell
                    .borrow_mut()
                    .enqueue(f.0 as u32)
                    .unwrap()
            },
            part.cell_size.0 as u32,
        );

        write_prod_fpg.precompensation = write_precompensation.0 as u32;

        if *track_data_to_write.borrow_has_non_flux_reversal_area() {
            write_prod_fpg.enable_non_flux_reversal_generator = true;
        } else {
            write_prod_fpg.enable_weak_bit_generator = true;
        }

        let mut track_data_iter = part.cells.iter();

        // prefill buffer with first data
        while self.write_prod_cell.borrow().len() < 70 {
            let mfm_byte = *track_data_iter.next().unwrap();
            to_bit_stream(mfm_byte, |bit| write_prod_fpg.feed(bit));
        }

        cortex_m::interrupt::free(|cs| {
            interrupts::FLUX_WRITER
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .prepare_transmit(cs);
        });

        // start transmit on index pulse
        cortex_m::interrupt::free(|cs| {
            START_TRANSMIT_ON_INDEX.borrow(cs).set(true);
        });

        if async_wait_for_transmit().await.is_err() {
            rprintln!("Transmit timeout? Drive not responsing.");
            return Err(RawTrackError::NoIndexPulse);
        }

        // continue until whole track is written.
        self.feed_mfm_raw_iterator_to_writer(track_data_iter, &mut write_prod_fpg)
            .await;

        for part in parts {
            let track_data_iter = part.cells.iter();

            write_prod_fpg.cell_duration = part.cell_size.0 as u32;
            self.feed_mfm_raw_iterator_to_writer(track_data_iter, &mut write_prod_fpg)
                .await;
        }

        /* Now this might be weird. We have to solve an issue here with our DMA.
         * At the moment, we erase the end of the track before writing to keep it clean
         * from any residual data. But this also means that we don't have any pulses
         * for reading after our groundtruth data. We need to add as many pulses here
         * as the reading DMA buffer is in length to fix this problem.
         * Otherwise reading will stall and slow us down for exactly one rotation of the disk.
         */
        for _ in 0..crate::flux_reader::BUFFER_SIZE {
            write_prod_fpg.feed(Bit(false));
            write_prod_fpg.feed(Bit(true));
        }

        write_prod_fpg.flush();

        Ok(track_data_to_write)
    }

    async fn verify_track(
        &mut self,
        track_data_to_write: RawCellData,
    ) -> Result<PulseDuration, (RawTrackError, RawCellData)> {
        // Size of sliding window, containing the significant data we use, trying
        // to match the data we read back against the groundtruth data we thought
        // to have written before
        const COMPARE_WINDOW_SIZE: usize = 20;

        // We record this amount of pulses to slide the COMPARE_WINDOW on
        // to perfom cross correlation
        const READ_DATA_WINDOW_SIZE: usize = 200;

        // keep the motor spinning
        cortex_m::interrupt::free(|cs| {
            interrupts::FLOPPY_CONTROL
                .borrow(cs)
                .borrow_mut()
                .as_mut()
                .unwrap()
                .spin_motor();
        });

        // Throw away all data in the queue before we read real data
        while self.read_cons.dequeue().is_some() {}

        // we might have multiple different cell densities. grab the first one
        let mut parts = track_data_to_write.borrow_parts().iter();
        let part = parts.next().unwrap();

        // How similar should the data be against the reference?
        // The minimum similarity is half of the bit cell. But we are better than that!
        // 35% should be ok!
        let similarity_treshold = part.cell_size.0 * 35 / 100;

        // prepare compare data around the first significant position to compare the data we read back to
        let flux_data_to_write_queue: RefCell<VecDeque<PulseDuration>> =
            RefCell::new(VecDeque::with_capacity(COMPARE_WINDOW_SIZE * 8));
        let mut flux_data_to_write_fpg = FluxPulseGenerator::new(
            |f| flux_data_to_write_queue.borrow_mut().push_back(f),
            part.cell_size.0 as u32,
        );

        if *track_data_to_write.borrow_has_non_flux_reversal_area() {
            // It is important to have the non flux reversal generator disabled here.
            // We will be reading an area of nothing after all!
            flux_data_to_write_fpg.enable_non_flux_reversal_generator = false;
        } else {
            flux_data_to_write_fpg.enable_weak_bit_generator = true;
        }

        let mut track_data_to_write_iter = part.cells.iter();

        let mut generate_ground_truth = || {
            while flux_data_to_write_queue.borrow().len() < COMPARE_WINDOW_SIZE {
                to_bit_stream(
                    *track_data_to_write_iter.next().unwrap_or_else(|| {
                        panic!("Not filled {}", flux_data_to_write_queue.borrow().len())
                    }),
                    |bit| flux_data_to_write_fpg.feed(bit),
                )
            }
        };
        generate_ground_truth();

        // start reception of track on next index pulse
        cortex_m::interrupt::free(|cs| {
            START_RECEIVE_ON_INDEX.borrow(cs).set(true);
        });

        if async_wait_for_receive().await.is_err() {
            return Err((RawTrackError::NoIndexPulse, track_data_to_write));
        };

        // remove the first 6 pulses from the groundtruth data to better
        // allow matching. Those 6 pulses are not verified but I guess that this is ok.
        for _ in 0..5 {
            flux_data_to_write_queue.borrow_mut().pop_front();
        }
        let last = flux_data_to_write_queue.borrow_mut().pop_front().unwrap();
        let mut removed = 6;

        // avoid lack of entropy by removing repeated data
        while flux_data_to_write_queue.borrow_mut().front().unwrap().0 == last.0 {
            removed += 1;
            flux_data_to_write_queue.borrow_mut().pop_front();

            // discard incoming value.
            if self.async_read_flux().await.is_none() {
                rprintln!("Timeout2");
                flux_reader_stop_reception();
                return Err((RawTrackError::NoIncomingData, track_data_to_write));
            };

            generate_ground_truth();
        }
        rprintln!("Remove repeated: {}", removed);
        generate_ground_truth();
        // reserve some memory for reading flux data from disk
        let mut read_mfm_flux_data_queue: VecDeque<PulseDuration> =
            VecDeque::with_capacity(READ_DATA_WINDOW_SIZE * 2);
        // now record something slightly larger than the "significant window"
        while read_mfm_flux_data_queue.len() < READ_DATA_WINDOW_SIZE {
            if let Some(pulse) = self.async_read_flux().await {
                read_mfm_flux_data_queue.push_back(PulseDuration(pulse))
            } else {
                rprintln!("Timeout2");
                flux_reader_stop_reception();
                return Err((RawTrackError::NoIncomingData, track_data_to_write));
            };
        }

        let mut equal = false; // set to true if correlation is found
        let mut match_after_pulses = 0;
        // now move the reference significant window over the already read data and compare it.
        // there should be one position where it matches!
        for read_window_index in 0..READ_DATA_WINDOW_SIZE {
            if read_mfm_flux_data_queue.len() < COMPARE_WINDOW_SIZE {
                rprintln!("Unable to cross correlate!");
                flux_reader_stop_reception();
                return Err((RawTrackError::NoCrossCorrelation, track_data_to_write));
            }
            equal = read_mfm_flux_data_queue
                .range(0..COMPARE_WINDOW_SIZE)
                .zip(flux_data_to_write_queue.borrow().iter())
                .all(|(x, y)| y.similar(x, similarity_treshold));

            if equal {
                match_after_pulses = read_window_index;
                break;
            }

            read_mfm_flux_data_queue.pop_front();
        }

        assert!(equal); // program flow check

        // We are now synchronized and shall compare upcoming data
        let mut maximum_diff = 0;
        let mut successful_compares = 0;

        let mut generate_groundtruth = || {
            if flux_data_to_write_queue.borrow().len() < 30 {
                if let Some(val) = track_data_to_write_iter.next() {
                    to_bit_stream(*val, |bit| flux_data_to_write_fpg.feed(bit))
                } else if let Some(part) = parts.next() {
                    flux_data_to_write_fpg.cell_duration = part.cell_size.0 as u32;

                    track_data_to_write_iter = part.cells.iter();
                } else {
                    flux_data_to_write_fpg.flush();
                }
            }
        };

        // we first need to get rid of the read_mfm_flux_data_queue before we read live data.
        // It slows down our processing if we continue to use this data structure
        loop {
            generate_groundtruth();

            if read_mfm_flux_data_queue.is_empty() {
                break;
            }

            let reference = flux_data_to_write_queue.borrow_mut().pop_front().unwrap();
            let readback = read_mfm_flux_data_queue.pop_front().unwrap();

            if reference.0 > part.cell_size.0 * 10 {
                // Non Flux Reversal Detected. Some cleanup needed.
                // TODO Is this really the best approach to fix this?
                // It is also pretty random. Sometimes it doesn't work at all.
                flux_data_to_write_queue.borrow_mut().pop_front().unwrap();
            } else if !reference.similar(&readback, similarity_treshold) {
                flux_reader_stop_reception();
                rprintln!(
                    "{} != {}, successful_compares until compare fail: {}",
                    reference.0,
                    readback.0,
                    successful_compares
                );

                return Err((RawTrackError::DataNotEqual, track_data_to_write));
            } else {
                maximum_diff = max(maximum_diff, (reference.0).abs_diff(readback.0));
            }
            successful_compares += 1;
        }

        mem::drop(read_mfm_flux_data_queue);

        // we got rid of the queue. Now do the same with live data until everything was verified.
        // TODO Copy pasta
        loop {
            generate_groundtruth();

            if flux_data_to_write_queue.borrow().is_empty() {
                break; // Yay! All is verified.
            }

            if let Some(readback) = self.read_cons.dequeue() {
                let reference = flux_data_to_write_queue.borrow_mut().pop_front().unwrap();

                // TODO Copy pasta
                if reference.0 > part.cell_size.0 * 10 {
                    // Non Flux Reversal Detected. Some cleanup needed.
                    // TODO Is this really the best approach to fix this?
                    // It is also pretty random. Sometimes it doesn't work at all.
                    flux_data_to_write_queue.borrow_mut().pop_front().unwrap();
                } else if !reference.similar(&PulseDuration(readback as i32), similarity_treshold) {
                    flux_reader_stop_reception();
                    rprintln!(
                        "{} != {}, successful_compares until compare fail: {}",
                        reference.0,
                        readback,
                        successful_compares
                    );

                    return Err((RawTrackError::DataNotEqual, track_data_to_write));
                } else {
                    maximum_diff = max(maximum_diff, (reference.0).abs_diff(readback as i32));
                }
                successful_compares += 1;
            } else {
                // We got CPU power to spare. Return from coroutine
                cassette::yield_now().await;
            }
        }

        flux_reader_stop_reception();
        rprintln!(
            "Verified {} pulses, max error {}/{}, match offset {}",
            successful_compares,
            maximum_diff,
            similarity_treshold,
            match_after_pulses
        );
        Ok(PulseDuration(maximum_diff as i32))
    }
}
