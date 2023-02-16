pub struct TrackConfiguration {
    pub cellsize: usize,
    pub sectors: u8,
    pub gap_size: usize,
}

#[must_use]
pub fn get_track_settings(track: usize) -> TrackConfiguration {
    assert_ne!(track, 0, "We are starting with 1 here!");
    if track <= 17 {
        TrackConfiguration {
            cellsize: 227,
            sectors: 21,
            gap_size: 8,
        }
    } else if track <= 24 {
        TrackConfiguration {
            cellsize: 245,
            sectors: 19,
            gap_size: 17,
        }
    } else if track <= 30 {
        TrackConfiguration {
            cellsize: 262,
            sectors: 18,
            gap_size: 12,
        }
    } else {
        TrackConfiguration {
            cellsize: 280,
            sectors: 17,
            gap_size: 9,
        }
    }
}
