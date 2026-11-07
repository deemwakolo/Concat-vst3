// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Latency and tail reporting for VST3 plugins.

/// The largest tail duration we will respect before treating it as effectively
/// infinite for host-side playback and export. Some VST3 plugins report
/// `kInfiniteTail` or a very large number; clamping keeps the host bounded and
/// prevents unbounded preview/export flush loops.
pub const MAX_SAFE_TAIL_SECONDS: f64 = 30.0;

/// Sums a plugin chain's reported latency without exceeding the caller's
/// processing bound.
pub fn bounded_chain_latency_frames(
    latencies: impl IntoIterator<Item = u32>,
    max_frames: usize,
) -> usize {
    latencies
        .into_iter()
        .fold(0usize, |total, frames| {
            total.saturating_add(frames as usize)
        })
        .min(max_frames)
}

/// Converts and sums reported tail durations at `sample_rate`, keeping
/// unknown and effectively infinite tails within `max_frames`.
pub fn bounded_chain_tail_frames(
    tails_seconds: impl IntoIterator<Item = f64>,
    sample_rate: u32,
    max_frames: usize,
) -> usize {
    tails_seconds
        .into_iter()
        .fold(0usize, |total, seconds| {
            let frames = if seconds.is_finite() {
                (seconds.max(0.0) * f64::from(sample_rate))
                    .ceil()
                    .min(max_frames as f64) as usize
            } else {
                0
            };
            total.saturating_add(frames)
        })
        .min(max_frames)
}

/// The plugin's algorithmic latency in samples.
///
/// The host must compensate for this by delaying the dry signal by the same
/// amount, or by shifting the plugin's output earlier by this many samples,
/// so that processed audio aligns with the timeline.
#[derive(Clone, Copy, Debug, Default)]
pub struct LatencyInfo {
    /// Latency in samples, at the plugin's initialized sample rate.
    pub samples: u32,
}

impl LatencyInfo {
    /// Converts the latency to seconds at the given sample rate.
    pub fn as_seconds(self, sample_rate: f64) -> f64 {
        if sample_rate > 0.0 {
            self.samples as f64 / sample_rate
        } else {
            0.0
        }
    }
}

/// The plugin's estimated tail time in seconds.
///
/// After the last input frame, the host should continue feeding silence for
/// at least this long so reverbs, delays and other tail effects can decay.
#[derive(Clone, Copy, Debug, Default)]
pub struct TailInfo {
    /// Estimated tail time in seconds.
    pub seconds: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_latency_is_cumulative_and_bounded() {
        assert_eq!(bounded_chain_latency_frames([12, 34, 56], 100), 100);
        assert_eq!(bounded_chain_latency_frames([12, 34], 100), 46);
    }

    #[test]
    fn chain_tails_ignore_unknown_values_and_respect_the_limit() {
        assert_eq!(
            bounded_chain_tail_frames([0.25, f64::INFINITY, f64::NAN, 1.0], 100, 100),
            100
        );
        assert_eq!(bounded_chain_tail_frames([0.25, 0.5], 100, 100), 75);
    }
}
