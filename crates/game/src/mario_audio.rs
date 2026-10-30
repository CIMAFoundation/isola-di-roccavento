//! Bounded PCM handoff from the main-thread native engine to Bevy's audio sink.
use bevy::{audio::{Decodable, Source}, prelude::*};
use std::{collections::VecDeque, sync::{Arc, Mutex}, time::Duration};

const MAX_SAMPLES: usize = libsm64_native::AUDIO_RATE as usize * 2 / 5;
const BLOCK_SAMPLES: usize = 512;

#[derive(Default)]
struct Queue {
    samples: VecDeque<i16>,
    active: bool,
    horn_frame: usize,
    horn: bool,
}

#[derive(Asset, TypePath, Clone, Default)]
pub(super) struct NativeAudio {
    queue: Arc<Mutex<Queue>>,
}

impl NativeAudio {
    pub fn start(&self) {
        let mut q = self.queue.lock().unwrap();
        q.samples.clear();
        q.horn = false;
        q.active = true;
    }
    pub fn stop(&self) {
        let mut q = self.queue.lock().unwrap();
        q.active = false;
        q.samples.clear();
        q.horn = false;
    }
    pub fn clear(&self) { let mut q = self.queue.lock().unwrap(); q.samples.clear(); q.horn = false; }
    #[cfg(test)]
    pub fn honk(&self) { let mut q = self.queue.lock().unwrap(); q.horn = true; q.horn_frame = 0; }
    pub fn queued_frames(&self) -> usize { self.queue.lock().unwrap().samples.len() / 2 }
    pub fn push(&self, samples: &[i16]) {
        let mut q = self.queue.lock().unwrap();
        if !q.active { return; }
        for frame in samples.chunks_exact(2) {
            let horn = if q.horn {
                let t = q.horn_frame as f32 / libsm64_native::AUDIO_RATE as f32;
                let envelope = (t / 0.015).min(1.) * ((0.3 - t) / 0.06).clamp(0., 1.);
                let value = ((t * 440. * std::f32::consts::TAU).sin()
                    + (t * 554. * std::f32::consts::TAU).sin()) * envelope * 4000.;
                q.horn_frame += 1;
                if t >= 0.3 { q.horn = false; }
                value as i16
            } else { 0 };
            q.samples.push_back(frame[0].saturating_add(horn));
            q.samples.push_back(frame[1].saturating_add(horn));
        }
        let excess = q.samples.len().saturating_sub(MAX_SAMPLES);
        q.samples.drain(..excess);
    }
}

pub(super) struct PcmDecoder {
    queue: Arc<Mutex<Queue>>,
    block: Vec<i16>,
    index: usize,
}

impl PcmDecoder {
    /// The PCM as libsm64 makes it, interleaved stereo.
    fn next_i16(&mut self) -> Option<i16> {
        if self.index >= self.block.len() {
            let mut q = self.queue.lock().unwrap();
            if !q.active { return None; }
            self.block.clear();
            let count = q.samples.len().min(BLOCK_SAMPLES);
            self.block.extend(q.samples.drain(..count));
            // Never block on native synthesis in the real-time callback.
            if self.block.is_empty() { self.block.resize(BLOCK_SAMPLES, 0); }
            self.index = 0;
        }
        let sample = self.block[self.index];
        self.index += 1;
        Some(sample)
    }
    #[cfg(test)]
    fn pcm(&mut self) -> impl Iterator<Item = i16> + '_ {
        std::iter::from_fn(move || self.next_i16())
    }
}

/// rodio (0.22, Bevy 0.20) plays `f32` samples only.
impl Iterator for PcmDecoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        self.next_i16().map(|s| f32::from(s) / 32768.0)
    }
}

impl Source for PcmDecoder {
    fn current_span_len(&self) -> Option<usize> { None }
    fn channels(&self) -> bevy::audio::ChannelCount { bevy::audio::ChannelCount::new(2).unwrap() }
    fn sample_rate(&self) -> bevy::audio::SampleRate { bevy::audio::SampleRate::new(libsm64_native::AUDIO_RATE).unwrap() }
    fn total_duration(&self) -> Option<Duration> { None }
}

impl Decodable for NativeAudio {
    type Decoder = PcmDecoder;
    fn decoder(&self) -> PcmDecoder {
        PcmDecoder { queue: self.queue.clone(), block: Vec::with_capacity(BLOCK_SAMPLES), index: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pcm_stereo_order_underflow_and_stop() {
        let source = NativeAudio::default();
        source.start();
        source.push(&[10, -10, 20, -20]);
        let mut decoder = source.decoder();
        assert_eq!(decoder.pcm().take(4).collect::<Vec<_>>(), [10, -10, 20, -20]);
        assert_eq!(decoder.next_i16(), Some(0));
        source.stop();
        for _ in 1..BLOCK_SAMPLES { assert_eq!(decoder.next_i16(), Some(0)); }
        assert_eq!(decoder.next_i16(), None);
        source.start();
        assert_eq!(source.queued_frames(), 0);
    }
    #[test]
    fn horn_mixes_in_stereo_without_clipping_or_extending_the_pcm_queue() {
        let source = NativeAudio::default();
        source.start();
        source.honk();
        source.push(&vec![32000; 3200]);
        let samples: Vec<_> = source.decoder().pcm().take(3200).collect();
        assert!(samples.iter().any(|&s| s == i16::MAX));
        assert!(samples.iter().any(|&s| s < 32000));
        assert!(samples.chunks_exact(2).all(|f| f[0] == f[1]));
        assert_eq!(source.queued_frames(), 0);
        source.push(&vec![0; 32000]);
        assert!(!source.queue.lock().unwrap().horn, "the horn finishes without another interaction");
        source.clear();
        source.push(&[42, -42]);
        assert_eq!(source.decoder().pcm().take(2).collect::<Vec<_>>(), [42, -42]);
    }
    #[test]
    fn a_missing_audio_device_cannot_accumulate_unbounded_latency() {
        let source = NativeAudio::default();
        source.start();
        source.push(&vec![1; MAX_SAMPLES]);
        source.push(&[2, 3]);
        assert_eq!(source.queued_frames(), MAX_SAMPLES / 2);
        let samples: Vec<_> = source.decoder().pcm().take(MAX_SAMPLES).collect();
        assert_eq!(&samples[MAX_SAMPLES - 2..], &[2, 3]);
    }
}
