// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Interleaved/planar conversion tests, plus a deterministic mock processor.
//! These run with and without the `native` feature.

use concat_vst3::buffer::{deinterleave, interleave};

fn run_mock_processor(input: &[f32], frames: usize, channels: usize, gain: f32) -> Vec<f32> {
    let total = frames * channels;
    let mut planar_in = vec![0.0f32; total];
    let mut planar_out = vec![0.0f32; total];
    let mut output = vec![0.0f32; total];

    deinterleave(&input[..total], frames, channels, &mut planar_in).unwrap();
    for (output_sample, input_sample) in planar_out.iter_mut().zip(planar_in.iter()) {
        *output_sample = *input_sample * gain;
    }
    interleave(&planar_out, frames, channels, &mut output).unwrap();
    output
}

#[test]
fn mono_interleaved_planar_interleaved_round_trip() {
    let input = [0.0f32, 0.25, -0.5, 1.0, -1.0];
    let output = run_mock_processor(&input, input.len(), 1, 1.0);
    assert_eq!(output, input);

    let mut planar = [0.0f32; 5];
    deinterleave(&input, 5, 1, &mut planar).unwrap();
    assert_eq!(planar, input);
}

#[test]
fn stereo_interleaved_planar_interleaved_round_trip() {
    let input = [0.1f32, -0.1, 0.2, -0.2, 0.3, -0.3];
    let output = run_mock_processor(&input, 3, 2, 1.0);
    assert_eq!(output, input);

    let mut planar = [0.0f32; 6];
    deinterleave(&input, 3, 2, &mut planar).unwrap();
    assert_eq!(planar, [0.1, 0.2, 0.3, -0.1, -0.2, -0.3]);

    let mut round_trip = [0.0f32; 6];
    interleave(&planar, 3, 2, &mut round_trip).unwrap();
    assert_eq!(round_trip, input);
}

#[test]
fn mock_gain_processing_is_sample_accurate() {
    let input = [0.1f32, -0.2, 0.3, -0.4, 0.5, -0.6, 0.7, -0.8];
    let output = run_mock_processor(&input, 4, 2, 0.5);
    let expected: Vec<f32> = input.iter().map(|sample| sample * 0.5).collect();
    for (actual, expected) in output.iter().zip(expected.iter()) {
        assert!((actual - expected).abs() < 1e-7);
    }
}

#[test]
fn arbitrary_valid_block_sizes_round_trip() {
    for frames in [0usize, 1, 2, 7, 64, 1024] {
        for channels in [1usize, 2] {
            let total = frames * channels;
            let input: Vec<f32> = (0..total)
                .map(|index| (index as f32 * 0.01).sin())
                .collect();
            let output = run_mock_processor(&input, frames, channels, 1.0);
            assert_eq!(output, input, "failed for {frames} frames, {channels} channels");
        }
    }
}

#[test]
fn invalid_layouts_are_rejected() {
    let mut planar = [0.0f32; 6];
    assert!(deinterleave(&[0.0; 5], 3, 2, &mut planar).is_err());
    assert!(deinterleave(&[0.0; 6], 3, 0, &mut planar).is_err());
    assert!(interleave(&[0.0; 5], 3, 2, &mut [0.0; 6]).is_err());
    assert!(interleave(&[0.0; 6], 3, 0, &mut [0.0; 6]).is_err());
}
