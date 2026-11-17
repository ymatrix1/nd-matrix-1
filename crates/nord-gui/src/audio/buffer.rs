//! `--audio-buffer`: the device buffer size the user asks for, clamped to the device range, with a
//! fallback to the driver default when the device rejects it.

/// Builds with the requested buffer size; when the device rejects it, says so and builds again
/// with the driver default so the app is never left silent by a bad `--audio-buffer`.
pub fn with_buffer_fallback<T, E: std::fmt::Display>(
    requested: Option<u32>,
    mut build: impl FnMut(Option<u32>) -> Result<T, E>,
) -> Result<T, E> {
    match (build(requested), requested) {
        (Err(e), Some(n)) => {
            eprintln!("[Audio] Device rejected a {n}-frame buffer ({e}); using the driver default");
            build(None)
        }
        (r, _) => r,
    }
}

/// `frames` limited to what the device reports it supports (`Unknown` keeps the request).
pub fn clamp_buffer(frames: u32, supported: &cpal::SupportedBufferSize) -> u32 {
    match supported {
        cpal::SupportedBufferSize::Range { min, max } => frames.clamp(*min, *max),
        cpal::SupportedBufferSize::Unknown => frames,
    }
}

/// `--audio-buffer <frames>`: fixed device buffer size; `None` keeps the driver default.
pub fn buffer_frames_from_args(args: &[String]) -> Option<u32> {
    let i = args.iter().position(|a| a == "--audio-buffer")?;
    args.get(i + 1)?.parse().ok().filter(|&n| n > 0)
}

/// `config` with the buffer size the user asked for.
pub fn stream_config(mut config: cpal::StreamConfig, frames: Option<u32>) -> cpal::StreamConfig {
    if let Some(n) = frames {
        config.buffer_size = cpal::BufferSize::Fixed(n);
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn audio_buffer_flag_parses_positive_frame_counts_only() {
        assert_eq!(buffer_frames_from_args(&args(&["x"])), None);
        let ok = args(&["x", "--audio-buffer", "512"]);
        assert_eq!(buffer_frames_from_args(&ok), Some(512));
        for bad in ["0", "big"] {
            assert_eq!(
                buffer_frames_from_args(&args(&["x", "--audio-buffer", bad])),
                None
            );
        }
        assert_eq!(
            buffer_frames_from_args(&args(&["x", "--audio-buffer"])),
            None
        );
    }

    #[test]
    fn stream_config_applies_the_fixed_buffer() {
        let base = cpal::StreamConfig {
            channels: 2,
            sample_rate: cpal::SampleRate(48_000),
            buffer_size: cpal::BufferSize::Default,
        };
        let kept = stream_config(base.clone(), None);
        assert!(matches!(kept.buffer_size, cpal::BufferSize::Default));
        let fixed = stream_config(base, Some(256));
        assert!(matches!(fixed.buffer_size, cpal::BufferSize::Fixed(256)));
    }

    #[test]
    fn a_rejected_buffer_size_falls_back_to_the_default() {
        let mut tried = Vec::new();
        let r = with_buffer_fallback(Some(64), |f| {
            tried.push(f);
            if f.is_some() {
                Err("size not supported".to_string())
            } else {
                Ok("stream")
            }
        });
        assert_eq!(r, Ok("stream"));
        assert_eq!(tried, [Some(64), None]);
    }

    #[test]
    fn fallback_builds_once_when_it_works_and_reports_a_failing_default() {
        let mut n = 0;
        let ok = with_buffer_fallback(Some(256), |_| {
            n += 1;
            Ok::<_, String>(())
        });
        assert!(ok.is_ok() && n == 1);
        let mut n = 0;
        let bad = with_buffer_fallback(None, |_| {
            n += 1;
            Err::<(), _>("no device")
        });
        assert_eq!((bad, n), (Err("no device"), 1));
    }

    #[test]
    fn buffer_request_is_clamped_to_the_device_range() {
        let range = cpal::SupportedBufferSize::Range { min: 32, max: 4096 };
        assert_eq!(clamp_buffer(8, &range), 32);
        assert_eq!(clamp_buffer(512, &range), 512);
        assert_eq!(clamp_buffer(99_999, &range), 4096);
        assert_eq!(clamp_buffer(7, &cpal::SupportedBufferSize::Unknown), 7);
    }
}
