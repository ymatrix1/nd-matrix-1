//! Alternation of the contenders of `lle-gates`: round by round every engine runs once, so slow
//! drift of the host (thermal state, other processes) hits all of them alike. Round 0 is the
//! warm-up and is discarded. The background load is sampled before the whole measurement, before
//! every run and after it.

use crate::load::LoadLog;
use nord_sharc::runner::uop_report::UopReport;

/// What one run of one contender observed.
#[derive(Clone, Debug, Default)]
pub struct Sample {
    /// Wall microseconds of every measured pass or tick of the run.
    pub us: Vec<f64>,
    /// Thread CPU microseconds of the same (wall where the platform has no thread counter).
    pub cpu_us: Vec<f64>,
    /// Counters of the measured passes of a uop engine.
    pub counters: Option<UopReport>,
    /// Every pass returned to its poll loop (SHARC) or stopped on its budget (ARM).
    pub steady: bool,
}

/// One engine or tier under test.
pub struct Contender {
    pub label: String,
    /// `false` for an engine whose run is not a timing (the checked engine allocates).
    pub timed: bool,
    pub run: Box<dyn FnMut() -> Result<Sample, String>>,
}

/// The measured runs of one contender.
#[derive(Clone, Debug, Default)]
pub struct Series {
    pub label: String,
    pub timed: bool,
    pub runs: Vec<Sample>,
    /// Background load sampled just before each measured run.
    pub load: Vec<Option<f64>>,
    /// Why there are no runs (unavailable engine, failed arming).
    pub error: Option<String>,
}

/// All series and the load log of one measurement.
#[derive(Clone, Debug, Default)]
pub struct Measured {
    pub series: Vec<Series>,
    pub load: LoadLog,
}

/// `rounds` measured rounds after one discarded warm-up round. `sampler` returns the background
/// load in percent (`None` when unknown).
pub fn alternate(
    contenders: &mut [Contender],
    rounds: usize,
    sampler: &dyn Fn() -> Option<f64>,
) -> Measured {
    let mut m = Measured {
        series: contenders
            .iter()
            .map(|c| Series {
                label: c.label.clone(),
                timed: c.timed,
                ..Series::default()
            })
            .collect(),
        load: LoadLog {
            before: sampler(),
            ..LoadLog::default()
        },
    };
    for round in 0..=rounds {
        for (c, s) in contenders.iter_mut().zip(m.series.iter_mut()) {
            if s.error.is_some() {
                continue;
            }
            let load = sampler();
            match (c.run)() {
                Err(e) => s.error = Some(e),
                Ok(_) if round == 0 => {}
                Ok(sample) => {
                    s.runs.push(sample);
                    s.load.push(load);
                    m.load.runs.push(load);
                }
            }
        }
    }
    m.load.after = sampler();
    m
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A contender that logs its label and returns the next of `us` as a one-sample run.
    pub fn scripted(label: &str, us: Vec<f64>, log: Rc<RefCell<Vec<String>>>) -> Contender {
        let (name, mut it) = (label.to_string(), us.into_iter());
        Contender {
            label: label.into(),
            timed: true,
            run: Box::new(move || {
                log.borrow_mut().push(name.clone());
                let v = it.next().ok_or("script exhausted")?;
                Ok(Sample {
                    us: vec![v],
                    cpu_us: vec![v],
                    steady: true,
                    ..Sample::default()
                })
            }),
        }
    }

    #[test]
    fn engines_alternate_round_by_round_and_the_first_round_is_discarded() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut cs = vec![
            scripted("a", vec![1.0, 2.0, 3.0], log.clone()),
            scripted("b", vec![10.0, 20.0, 30.0], log.clone()),
        ];
        let m = alternate(&mut cs, 2, &|| Some(1.0));
        assert_eq!(*log.borrow(), ["a", "b", "a", "b", "a", "b"]);
        let a: Vec<f64> = m.series[0].runs.iter().map(|r| r.us[0]).collect();
        assert_eq!(a, [2.0, 3.0], "the warm-up value 1.0 is dropped");
        assert_eq!(m.series[1].load, vec![Some(1.0); 2]);
        assert_eq!(m.load.runs.len(), 4);
        assert_eq!((m.load.before, m.load.after), (Some(1.0), Some(1.0)));
    }

    #[test]
    fn a_failing_contender_stops_running_and_keeps_its_reason_while_the_others_go_on() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut cs = vec![
            scripted("a", vec![1.0], log.clone()), // exhausted in round 1
            scripted("b", vec![1.0, 2.0, 3.0], log.clone()),
        ];
        let m = alternate(&mut cs, 2, &|| None);
        assert_eq!(m.series[0].error.as_deref(), Some("script exhausted"));
        assert!(m.series[0].runs.is_empty());
        assert_eq!(m.series[1].runs.len(), 2);
        assert_eq!(
            log.borrow().iter().filter(|l| *l == "a").count(),
            2,
            "no run after the failure"
        );
        assert!(
            !m.load.valid(25.0),
            "an unknown load invalidates the timings"
        );
    }
}
