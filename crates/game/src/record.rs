//! Video recording (user, 2026-10-30), for demos: a `KIOSK_SNAP` state with
//! `rec=<seconds>` is filmed instead of photographed, picture and sound, into
//! `<KIOSK_SNAP>/<name>.mp4` (needs `ffmpeg` on the PATH).
//!
//! The game runs on a fixed clock while filming: every frame is 1/[`FPS`] s
//! of game time however long it takes to draw, so the video is smooth on any
//! machine and nothing is dropped. The window stays hidden, as for the
//! snapshots. The picture of every frame is piped to ffmpeg in order; the
//! sound is mixed by `sound::record_audio` from what the game plays (the same
//! sources, volumes and timing, at the game's clock) and muxed at the end.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether a recording is under way.
static RUNNING: AtomicBool = AtomicBool::new(false);

pub fn running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

/// No recording in the browser.
#[cfg(target_arch = "wasm32")]
#[derive(bevy::prelude::Resource, Default)]
pub struct Recorder;

#[cfg(target_arch = "wasm32")]
impl Recorder {
    pub fn start(&mut self, _: &mut bevy::prelude::Commands, _: &std::path::Path, _: f32) {}
    pub fn frame(&mut self, _: &mut bevy::prelude::Commands, _: &bevy::prelude::Handle<bevy::prelude::Image>) -> bool {
        true
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use bevy::prelude::*;
    use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
    use bevy::time::TimeUpdateStrategy;

    use super::RUNNING;

    /// Frames per second of the video, and of the game's clock while filming.
    pub const FPS: u32 = 30;
    /// The sound's sample rate (`sfx::RATE`).
    pub const RATE: u32 = sfx::RATE;

    /// The recording under way, if any.
    #[derive(Resource, Default)]
    pub struct Recorder {
        job: Option<Job>,
    }

    struct Job {
        path: PathBuf,
        frames: u32,
        /// Pictures asked for so far.
        asked: u32,
        video: Arc<Mutex<Video>>,
        /// The sound so far, stereo, interleaved, at [`RATE`].
        audio: Vec<f32>,
        /// Sound samples owed to the next frame (fractions carry over).
        owed: f64,
    }

    /// The pictures on their way to ffmpeg: they may come back out of order.
    struct Video {
        next: u32,
        pending: BTreeMap<u32, Vec<u8>>,
        ffmpeg: Option<(Child, ChildStdin)>,
        size: (u32, u32),
        failed: bool,
    }

    impl Recorder {
        /// Starts filming `seconds` into `<path>.mp4`.
        pub fn start(&mut self, commands: &mut Commands, path: &Path, seconds: f32) {
            let frames = (seconds * FPS as f32).round().max(1.0) as u32;
            self.job = Some(Job {
                path: path.to_owned(),
                frames,
                asked: 0,
                video: Arc::new(Mutex::new(Video { next: 0, pending: BTreeMap::new(), ffmpeg: None, size: (0, 0), failed: false })),
                audio: Vec::new(),
                owed: 0.0,
            });
            commands.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / f64::from(FPS))));
            RUNNING.store(true, Ordering::Relaxed);
            eprintln!("rec: {} ({frames} frames)", path.display());
        }

        /// No sound recorded yet: the recording has just begun.
        pub fn fresh(&self) -> bool {
            self.job.as_ref().is_some_and(|j| j.audio.is_empty())
        }

        /// The sound of this frame: `dt` seconds of game time, which
        /// `mix(left_right_buffer)` fills (it starts silent).
        pub fn audio(&mut self, dt: f32, mix: impl FnOnce(&mut [f32])) {
            let Some(job) = &mut self.job else { return };
            if job.asked >= job.frames {
                return;
            }
            job.owed += f64::from(dt) * f64::from(RATE);
            let n = job.owed.floor() as usize;
            job.owed -= n as f64;
            let start = job.audio.len();
            job.audio.resize(start + 2 * n, 0.0);
            mix(&mut job.audio[start..]);
        }

        /// Asks for this frame's picture; true once the recording is written.
        pub fn frame(&mut self, commands: &mut Commands, target: &Handle<Image>) -> bool {
            let Some(job) = &mut self.job else { return true };
            if job.asked < job.frames {
                let index = job.asked;
                let video = job.video.clone();
                // `KIOSK_REC_PNG`: every frame also as a lossless picture,
                // `<name>_frames/f0000.png`, for frame-by-frame comparisons
                // (the video's B-frames blur one frame in two)
                let png = std::env::var_os("KIOSK_REC_PNG").map(|_| PathBuf::from(format!("{}_frames", job.path.display())));
                commands.spawn(Screenshot::image(target.clone())).observe(move |shot: On<ScreenshotCaptured>| {
                    let Ok(img) = shot.image.clone().try_into_dynamic() else { return };
                    let rgba = img.to_rgba8();
                    if let Some(dir) = &png {
                        let _ = std::fs::create_dir_all(dir);
                        if let Err(e) = rgba.save(dir.join(format!("f{index:04}.png"))) {
                            eprintln!("rec: {}: {e}", dir.display());
                        }
                    }
                    let mut v = video.lock().unwrap();
                    if v.size == (0, 0) {
                        v.size = rgba.dimensions();
                    }
                    v.pending.insert(index, rgba.into_raw());
                    v.flush();
                });
                job.asked += 1;
                return false;
            }
            let (done, failed) = {
                let v = job.video.lock().unwrap();
                (v.next >= job.frames, v.failed)
            };
            if !done && !failed {
                return false;
            }
            let job = self.job.take().expect("a job");
            commands.insert_resource(TimeUpdateStrategy::Automatic);
            RUNNING.store(false, Ordering::Relaxed);
            let path = job.path.clone();
            if let Err(e) = job.finish() {
                eprintln!("rec: {}: {e}", path.display());
            }
            true
        }
    }

    impl Video {
        /// Sends the pictures that are next in line to ffmpeg.
        fn flush(&mut self) {
            while let Some(px) = self.pending.remove(&self.next) {
                if self.ffmpeg.is_none() && !self.failed {
                    match spawn_ffmpeg(self.size) {
                        Ok(f) => self.ffmpeg = Some(f),
                        Err(e) => {
                            eprintln!("rec: ffmpeg: {e}");
                            self.failed = true;
                        }
                    }
                }
                if let Some((_, stdin)) = &mut self.ffmpeg {
                    if let Err(e) = stdin.write_all(&px) {
                        eprintln!("rec: ffmpeg: {e}");
                        self.failed = true;
                    }
                }
                self.next += 1;
            }
        }
    }

    fn video_path() -> PathBuf {
        std::env::temp_dir().join(format!("rocca_rec_{}.mp4", std::process::id()))
    }

    fn spawn_ffmpeg((w, h): (u32, u32)) -> std::io::Result<(Child, ChildStdin)> {
        let mut child = Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-r", &FPS.to_string(), "-i", "-"])
            .args(["-c:v", "libx264", "-crf", "18", "-preset", "medium", "-pix_fmt", "yuv420p"])
            .arg(video_path())
            .stdin(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().expect("piped");
        Ok((child, stdin))
    }

    impl Job {
        /// Closes the picture, writes the sound and puts the two together.
        fn finish(self) -> Result<(), String> {
            let ffmpeg = self.video.lock().unwrap().ffmpeg.take();
            let (mut child, stdin) = ffmpeg.ok_or("no picture")?;
            drop(stdin);
            child.wait().map_err(|e| e.to_string())?;
            let wav = std::env::temp_dir().join(format!("rocca_rec_{}.wav", std::process::id()));
            write_wav(&wav, &self.audio).map_err(|e| e.to_string())?;
            // not `with_extension`: a name may hold a dot ("x1.25")
            let out = std::path::PathBuf::from(format!("{}.mp4", self.path.display()));
            let status = Command::new("ffmpeg")
                .args(["-loglevel", "error", "-y", "-i"])
                .arg(video_path())
                .arg("-i")
                .arg(&wav)
                .args(["-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-shortest", "-movflags", "+faststart"])
                .arg(&out)
                .status()
                .map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(video_path());
            let _ = std::fs::remove_file(&wav);
            if !status.success() {
                return Err(format!("ffmpeg: {status}"));
            }
            eprintln!("rec: {} ({} frames, {:.1} s of sound)", out.display(), self.frames, self.audio.len() as f32 / 2.0 / RATE as f32);
            Ok(())
        }
    }

    /// 16-bit stereo WAV.
    fn write_wav(path: &Path, samples: &[f32]) -> std::io::Result<()> {
        let data = (samples.len() * 2) as u32;
        let mut b = Vec::with_capacity(44 + data as usize);
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&RATE.to_le_bytes());
        b.extend_from_slice(&(RATE * 4).to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        for s in samples {
            b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        std::fs::write(path, b)
    }
}
