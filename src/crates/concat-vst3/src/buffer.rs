// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Interleaved/planar sample conversion used by the VST3 processing path.
//!
//! Concat's mix buffers are interleaved (`L R L R ...`), while VST3's
//! `AudioBusBuffers::channelBuffers32` is planar (one contiguous buffer per
//! channel). These helpers are deliberately allocation-free: callers reuse
//! their scratch buffers.

use crate::errors::Error;

/// Deinterleaves `frames * channels` samples from `interleaved` into planar
/// channel-major order in `planar`.
///
/// `planar` must be at least `frames * channels` samples long. The first
/// `frames` samples are channel 0, the next `frames` are channel 1, and so on.
pub fn deinterleave(
    interleaved: &[f32],
    frames: usize,
    channels: usize,
    planar: &mut [f32],
) -> Result<(), Error> {
    let total = checked_layout(frames, channels, interleaved.len(), planar.len())?;
    for (frame_index, frame) in interleaved[..total].chunks_exact(channels).enumerate() {
        for channel in 0..channels {
            planar[channel * frames + frame_index] = frame[channel];
        }
    }
    Ok(())
}

/// Interleaves planar channel-major `planar` samples into `interleaved`.
///
/// `interleaved` must be at least `frames * channels` samples long.
pub fn interleave(
    planar: &[f32],
    frames: usize,
    channels: usize,
    interleaved: &mut [f32],
) -> Result<(), Error> {
    let total = checked_layout(frames, channels, planar.len(), interleaved.len())?;
    for frame_index in 0..frames {
        for channel in 0..channels {
            interleaved[frame_index * channels + channel] = planar[channel * frames + frame_index];
        }
    }
    debug_assert_eq!(total, frames * channels);
    Ok(())
}

fn checked_layout(
    frames: usize,
    channels: usize,
    first_len: usize,
    second_len: usize,
) -> Result<usize, Error> {
    if channels == 0 {
        return Err(Error::Unsupported(
            "channel count must be at least one".to_owned(),
        ));
    }
    let total = frames.checked_mul(channels).ok_or_else(|| {
        Error::Unsupported(format!(
            "frame/channel layout overflows: {frames} frames x {channels} channels"
        ))
    })?;
    if first_len < total || second_len < total {
        return Err(Error::Processing(format!(
            "audio buffer too small: need {total} samples, got {first_len} and {second_len}"
        )));
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_round_trip_is_identity() {
        let input = [0.25f32, -0.5, 1.0];
        let mut planar = [0.0f32; 3];
        let mut output = [0.0f32; 3];

        deinterleave(&input, 3, 1, &mut planar).unwrap();
        interleave(&planar, 3, 1, &mut output).unwrap();

        assert_eq!(planar, input);
        assert_eq!(output, input);
    }

    #[test]
    fn stereo_round_trip_is_identity() {
        let input = [0.1f32, 0.2, 0.3, 0.4, 0.5, 0.6];
        let mut planar = [0.0f32; 6];
        let mut output = [0.0f32; 6];

        deinterleave(&input, 3, 2, &mut planar).unwrap();
        assert_eq!(planar, [0.1, 0.3, 0.5, 0.2, 0.4, 0.6]);

        interleave(&planar, 3, 2, &mut output).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn rejects_short_buffers_and_zero_channels() {
        let mut planar = [0.0f32; 4];
        assert!(deinterleave(&[0.0; 5], 3, 2, &mut planar).is_err());
        assert!(deinterleave(&[0.0; 6], 3, 0, &mut planar).is_err());
        assert!(interleave(&[0.0; 5], 3, 2, &mut [0.0; 6]).is_err());
    }
}
