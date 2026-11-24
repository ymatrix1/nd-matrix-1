//! Telemetry snapshot (OLED framebuffers, LED state) and its lock-free single-writer /
//! multi-reader channel (bean `ns4-lle6`).
//!
//! The engine thread owns the [`TelemetryWriter`] (not `Clone`: single writer by construction)
//! and publishes once per frame or block; the GUI clones [`TelemetryReader`]s and polls. Publish
//! and read copy into caller-provided buffers on the stack: no allocation, no lock. Transport:
//! [`SeqCell`].

use super::seqlock::SeqCell;
use std::sync::Arc;

/// Number of OLED panels (Program and Synth displays).
/// VERIFIED: two SSD1309-class 128 x 64 OLEDs over EMIFA CS2, `docs/research/arm-oled-render.md`
/// (`tools/arm_panel.py capture`).
pub const OLED_COUNT: usize = 2;

/// Bytes of one OLED framebuffer, 128 x 64 at 1 bit per pixel.
/// VERIFIED: framebuffer `[display + 0x18]`, 1024 bytes, same research note, bean `ns4-2gdc`
/// criterion "frame buffer / flush found".
pub const OLED_BYTES: usize = 128 * 64 / 8;

/// LED state words (1 bit per LED).
/// ASSUMED: 512 LEDs bound the panel (the inventory lists 274 controls, many with LED ladders);
/// the engine wiring (`ns4-lle5`) fixes the real bit map, only the size here would change.
pub const LED_WORDS: usize = 16;

/// Words in the seqlock payload: 4 framebuffer bytes per word, LED words, frame counter, ARM time.
pub const WORDS: usize = OLED_COUNT * OLED_BYTES / 4 + LED_WORDS + 2;

/// One consistent view of the emulated panel outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetrySnapshot {
    /// Framebuffers as the controller RAM holds them: byte `page * 128 + x`, bit `i` (LSB first)
    /// = row `page * 8 + i`.
    /// VERIFIED: the firmware flush `0xC0076924` sends 1024 bytes from `[display + 0x18]` in this
    /// order and every draw routine uses it, `docs/research/arm-oled-render.md` s2.3, checked by
    /// `tools/arm_panel.py capture` (the Rust test `led_bits_and_oled_pixels` only proves our
    /// accessor matches this layout). ASSUMED: the glass shows RAM upright (same note, s2.3).
    pub oled: [[u8; OLED_BYTES]; OLED_COUNT],
    /// LED bit map (bit `n` of the flat array = LED `n`).
    pub leds: [u32; LED_WORDS],
    /// Frames published so far by the engine (wraps).
    pub frame: u32,
    /// ARM0 virtual time in milliseconds when the snapshot was taken (wraps).
    pub arm_ms: u32,
}

impl Default for TelemetrySnapshot {
    fn default() -> Self {
        Self {
            oled: [[0; OLED_BYTES]; OLED_COUNT],
            leds: [0; LED_WORDS],
            frame: 0,
            arm_ms: 0,
        }
    }
}

impl TelemetrySnapshot {
    /// State of LED `n` (`false` out of range).
    pub fn led(&self, n: usize) -> bool {
        self.leds
            .get(n / 32)
            .is_some_and(|w| (w >> (n % 32)) & 1 == 1)
    }

    /// Sets LED `n` (ignored out of range).
    pub fn set_led(&mut self, n: usize, on: bool) {
        if let Some(w) = self.leds.get_mut(n / 32) {
            let bit = 1u32 << (n % 32);
            if on {
                *w |= bit;
            } else {
                *w &= !bit;
            }
        }
    }

    /// RAM pixel `(x, y)` of OLED `which` under the page layout above (`false` out of range).
    pub fn oled_pixel(&self, which: usize, x: usize, y: usize) -> bool {
        if which >= OLED_COUNT || x >= 128 || y >= 64 {
            return false;
        }
        (self.oled[which][(y / 8) * 128 + x] >> (y % 8)) & 1 == 1
    }

    fn to_words(&self, w: &mut [u32; WORDS]) {
        let mut i = 0;
        for fb in &self.oled {
            for k in 0..OLED_BYTES / 4 {
                let c = &fb[k * 4..k * 4 + 4];
                w[i] = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                i += 1;
            }
        }
        w[i..i + LED_WORDS].copy_from_slice(&self.leds);
        w[i + LED_WORDS] = self.frame;
        w[i + LED_WORDS + 1] = self.arm_ms;
    }

    fn load_words(&mut self, w: &[u32; WORDS]) {
        let mut i = 0;
        for fb in &mut self.oled {
            for k in 0..OLED_BYTES / 4 {
                fb[k * 4..k * 4 + 4].copy_from_slice(&w[i].to_le_bytes());
                i += 1;
            }
        }
        self.leds.copy_from_slice(&w[i..i + LED_WORDS]);
        self.frame = w[i + LED_WORDS];
        self.arm_ms = w[i + LED_WORDS + 1];
    }
}

/// Engine end: the only thread that may publish.
pub struct TelemetryWriter {
    cell: Arc<SeqCell<WORDS>>,
}

/// GUI end: cheap to clone, any number of readers.
#[derive(Clone)]
pub struct TelemetryReader {
    cell: Arc<SeqCell<WORDS>>,
}

/// Creates the channel (one allocation, here).
pub fn channel() -> (TelemetryWriter, TelemetryReader) {
    let cell = Arc::new(SeqCell::new());
    (
        TelemetryWriter {
            cell: Arc::clone(&cell),
        },
        TelemetryReader { cell },
    )
}

impl TelemetryWriter {
    /// Publishes `snap`. Wait-free, allocation-free.
    pub fn publish(&mut self, snap: &TelemetrySnapshot) {
        let mut w = [0u32; WORDS];
        snap.to_words(&mut w);
        self.cell.publish(&w);
    }
}

impl TelemetryReader {
    /// Copies the newest consistent snapshot into `out`; `false` before the first publish.
    pub fn read(&self, out: &mut TelemetrySnapshot) -> bool {
        let mut w = [0u32; WORDS];
        if !self.cell.read(&mut w) {
            return false;
        }
        out.load_words(&w);
        true
    }

    /// Completed publishes so far.
    pub fn publishes(&self) -> u32 {
        self.cell.publishes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn stamped(n: u32) -> TelemetrySnapshot {
        let mut s = TelemetrySnapshot::default();
        s.oled[0] = [n as u8; OLED_BYTES];
        s.oled[1] = [!(n as u8); OLED_BYTES];
        s.leds = [n; LED_WORDS];
        s.frame = n;
        s.arm_ms = n.wrapping_mul(3);
        s
    }

    fn consistent(s: &TelemetrySnapshot) -> bool {
        let n = s.frame;
        s.oled[0].iter().all(|&b| b == n as u8)
            && s.oled[1].iter().all(|&b| b == !(n as u8))
            && s.leds.iter().all(|&l| l == n)
            && s.arm_ms == n.wrapping_mul(3)
    }

    #[test]
    fn words_round_trip_every_field() {
        let mut s = stamped(0x1234_5678);
        s.oled[0][0] = 1;
        s.oled[1][OLED_BYTES - 1] = 9;
        let mut w = [0u32; WORDS];
        s.to_words(&mut w);
        let mut back = TelemetrySnapshot::default();
        back.load_words(&w);
        assert_eq!(back, s);
    }

    #[test]
    fn reader_sees_nothing_then_the_latest_snapshot() {
        let (mut w, r) = channel();
        let mut out = TelemetrySnapshot::default();
        assert!(!r.read(&mut out));
        w.publish(&stamped(1));
        w.publish(&stamped(2));
        let r2 = r.clone();
        assert!(r2.read(&mut out));
        assert_eq!(out, stamped(2));
        assert_eq!(r.publishes(), 2);
    }

    #[test]
    fn led_bits_and_oled_pixels() {
        let mut s = TelemetrySnapshot::default();
        s.set_led(0, true);
        s.set_led(37, true);
        s.set_led(1000, true); // out of range: ignored
        assert!(s.led(0) && s.led(37) && !s.led(1) && !s.led(1000));
        s.set_led(37, false);
        assert!(!s.led(37));
        // Column 5, rows 8..16 live in byte 128 + 5; bit 3 is row 11.
        s.oled[1][128 + 5] = 1 << 3;
        assert!(s.oled_pixel(1, 5, 11));
        assert!(!s.oled_pixel(1, 5, 10) && !s.oled_pixel(0, 5, 11));
        assert!(!s.oled_pixel(2, 0, 0) && !s.oled_pixel(0, 128, 0) && !s.oled_pixel(0, 0, 64));
    }

    #[test]
    fn concurrent_readers_never_see_a_torn_snapshot() {
        let (mut w, r) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_w = Arc::clone(&stop);
        let writer = std::thread::spawn(move || {
            let mut n = 1u32;
            while !stop_w.load(Ordering::Relaxed) {
                w.publish(&stamped(n));
                n = n.wrapping_add(1).max(1);
            }
            n
        });
        let readers: Vec<_> = (0..3)
            .map(|_| {
                let r = r.clone();
                std::thread::spawn(move || {
                    let (mut ok, mut out) = (0u32, TelemetrySnapshot::default());
                    let t0 = std::time::Instant::now();
                    while t0.elapsed() < std::time::Duration::from_millis(300) {
                        if r.read(&mut out) {
                            assert!(consistent(&out), "torn snapshot at frame {}", out.frame);
                            ok += 1;
                        }
                    }
                    ok
                })
            })
            .collect();
        let reads: u32 = readers.into_iter().map(|h| h.join().unwrap()).sum();
        stop.store(true, Ordering::Relaxed);
        assert!(writer.join().unwrap() > 1);
        assert!(reads > 0, "no consistent read at all");
    }
}
