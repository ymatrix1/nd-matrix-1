//! Lane-to-chain feed stand-in (`sharc-sys --lane S:P --feed S:P:DST[:v|p]`, D172).
//!
//! On the live PNO core 1 pass nothing writes the effect chain input pair `DM(0xC2E06)`, so a
//! hand-linked Leslie element runs on silence (D171). This runner copies the dry output lane of
//! the previous pass into a destination pair before every pass and records both the lane and the
//! destination after the pass, so the chain output can be compared with the lane (one pass
//! latency). It is an experiment harness, not a model of the hardware hand-over.
use super::msg::{deliver, Msg};
use super::pass::PassConfig;
use super::periph_run::lane_words;
use super::system::{lean_pass, set_ping_pong};
use crate::core::Core;

/// How the four pair words `[a b c d]` are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Lane order `[L0 R0 L1 R1]`, copied as is.
    Verbatim,
    /// Chain order `[L0 L1 R0 R1]` (left at `+0`, right at `+2` as the chain loop reads it).
    Planar,
}

/// A feed: source lane `(set, pair)`, destination address, destination layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Feed {
    pub src: (u32, u32),
    pub dst: u32,
    pub layout: Layout,
}

/// Parses `SET:PAIR:DST[:v|p]` (numbers decimal or `0x`; layout `v` verbatim, `p` planar).
pub fn parse_feed(s: &str) -> Result<Feed, String> {
    let bad = || format!("bad --feed {s:?} (want SET:PAIR:DST[:v|p])");
    let parts: Vec<&str> = s.split(':').collect();
    if !(3..=4).contains(&parts.len()) {
        return Err(bad());
    }
    let num = |t: &str| super::parse_number(t).ok_or_else(bad);
    let (set, pair, dst) = (num(parts[0])?, num(parts[1])?, num(parts[2])?);
    if set > 5 || pair > 2 {
        return Err(bad());
    }
    let layout = match parts.get(3).copied() {
        None | Some("p") => Layout::Planar,
        Some("v") => Layout::Verbatim,
        Some(_) => return Err(bad()),
    };
    Ok(Feed {
        src: (set, pair),
        dst,
        layout,
    })
}

/// Splits the four words `[w0 w1 w2 w3]` of a pair into `(left, right)` frame pairs.
fn split(w: [u32; 4], layout: Layout) -> ([f32; 2], [f32; 2]) {
    let f = f32::from_bits;
    match layout {
        Layout::Verbatim => ([f(w[0]), f(w[2])], [f(w[1]), f(w[3])]),
        Layout::Planar => ([f(w[0]), f(w[1])], [f(w[2]), f(w[3])]),
    }
}

/// Joins a lane pair `[L0 R0 L1 R1]` into the destination layout.
fn arrange(lane: [u32; 4], layout: Layout) -> [u32; 4] {
    match layout {
        Layout::Verbatim => lane,
        Layout::Planar => [lane[0], lane[2], lane[1], lane[3]],
    }
}

/// Output of [`feed_run`]: the lane and the destination pair, both as two frames per pass.
#[derive(Debug, Default, PartialEq)]
pub struct FeedRun {
    pub lane: [Vec<f32>; 2],
    pub chain: [Vec<f32>; 2],
    /// One recorded pair per extra address, in the destination layout.
    pub extra: Vec<[Vec<f32>; 2]>,
    pub passes: u32,
}

fn pair_words(core: &mut Core, base: u32) -> [u32; 4] {
    let mut w = [0u32; 4];
    for (k, s) in w.iter_mut().enumerate() {
        *s = core.dm_read(base + k as u32).unwrap_or(0);
    }
    w
}

/// Runs `total` passes; before each pass the previous pass's lane pair is written to `feed.dst`.
pub fn feed_run(
    core: &mut Core,
    cfg: &PassConfig,
    msgs: &[Msg],
    total: u32,
    ping_pong: Option<(u32, u32, u32)>,
    feed: &Feed,
    extra: &[u32],
) -> Result<FeedRun, String> {
    let mut due: Vec<&Msg> = msgs.iter().collect();
    due.sort_by_key(|m| m.pass);
    let (mut next, mut out) = (0, FeedRun::default());
    out.extra = extra.iter().map(|_| Default::default()).collect();
    let mut prev = [0u32; 4];
    for p in 0..total {
        if let Some(pp) = ping_pong {
            set_ping_pong(core, pp, p);
        }
        while next < due.len() && due[next].pass <= p {
            deliver(core, 0, due[next])?;
            next += 1;
        }
        for (k, w) in arrange(prev, feed.layout).into_iter().enumerate() {
            let _ = core.dm_write(feed.dst + k as u32, w);
        }
        if !lean_pass(core, cfg).1 {
            break;
        }
        let lw = lane_words(core, feed.src.0);
        let b = 4 * feed.src.1 as usize;
        prev = [lw[b], lw[b + 1], lw[b + 2], lw[b + 3]];
        let (l, r) = split(prev, Layout::Verbatim);
        out.lane[0].extend(l);
        out.lane[1].extend(r);
        let (l, r) = split(pair_words(core, feed.dst), feed.layout);
        out.chain[0].extend(l);
        out.chain[1].extend(r);
        for (slot, &addr) in out.extra.iter_mut().zip(extra) {
            let (l, r) = split(pair_words(core, addr), feed.layout);
            slot[0].extend(l);
            slot[1].extend(r);
        }
        out.passes += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::pass::tests::clearing_core;
    use super::*;

    #[test]
    fn parse_feed_reads_lane_destination_and_layout() {
        let f = parse_feed("2:0:0xC2E06").unwrap();
        assert_eq!((f.src, f.dst, f.layout), ((2, 0), 0xC2E06, Layout::Planar));
        assert_eq!(parse_feed("1:2:16:v").unwrap().layout, Layout::Verbatim);
        for bad in ["", "2:0", "6:0:1", "2:3:1", "2:0:1:x", "a:0:1", "1:2:3:p:q"] {
            assert!(parse_feed(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn arrange_and_split_agree_on_the_two_layouts() {
        let lane = [1, 2, 3, 4]; // L0 R0 L1 R1
        assert_eq!(arrange(lane, Layout::Verbatim), lane);
        assert_eq!(arrange(lane, Layout::Planar), [1, 3, 2, 4]);
        let w = [1f32, 2., 3., 4.].map(f32::to_bits);
        assert_eq!(split(w, Layout::Verbatim), ([1., 3.], [2., 4.]));
        assert_eq!(split(w, Layout::Planar), ([1., 2.], [3., 4.]));
    }

    #[test]
    fn feed_run_records_the_lane_and_the_destination_after_the_pass() {
        let mut c = clearing_core();
        c.dm_write(0xB_0159 + 2, 0xB_0200).unwrap();
        for (k, v) in [0.5f32, -0.25, 0.125, 1.0].into_iter().enumerate() {
            c.dm_write(0xB_0200 + k as u32, v.to_bits()).unwrap();
        }
        let cfg = PassConfig {
            trigger: 0xB_00E2,
            idle_pc: 0x12_0002,
            max_steps: 10,
            irq: None,
        };
        let feed = parse_feed("2:0:0xB0300:v").unwrap();
        let r = feed_run(&mut c, &cfg, &[], 3, None, &feed, &[0xB_0200]).unwrap();
        // the tiny core idles once (see the lane_run test): the first pass feeds silence, so the
        // destination holds zeros while the lane holds the staged words
        assert_eq!(r.passes, 1);
        assert_eq!(r.lane[0], vec![0.5, 0.125]);
        assert_eq!(r.lane[1], vec![-0.25, 1.0]);
        assert_eq!(r.chain[0], vec![0.0, 0.0]);
        assert_eq!(
            r.extra[0][0],
            vec![0.5, -0.25],
            "extra pair, verbatim layout"
        );
    }
}
