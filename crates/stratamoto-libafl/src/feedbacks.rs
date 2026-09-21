//! What makes a run a finding, beyond a crash of the VM: a timeout confirmed as a hang, and
//! a report from the scenario, filed by the oracle that made it.

use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
    fmt::Debug,
    marker::PhantomData,
    path::{Path, PathBuf},
    rc::Rc,
};

use libafl::{
    HasMetadata,
    corpus::Testcase,
    events::{Event, EventFirer, EventWithStats},
    executors::ExitKind,
    feedbacks::{Feedback, StateInitializer},
    inputs::Input,
    monitors::stats::{AggregatorOps, UserStats, UserStatsValue},
    observers::{ObserversTuple, StdOutObserver},
    state::{HasCorpus, HasExecutions},
};
use libafl_bolts::{
    Error, Named,
    tuples::{Handle, MatchNameRef},
};
use regex::bytes::Regex;

use crate::{input::IrInput, stages::TimeoutsToVerify};

/// A feedback that captures all timeouts and stores them in the state for re-evaluation
/// later. Used with `VerifyTimeoutsStage`.
#[derive(Debug)]
pub struct CaptureTimeoutFeedback {
    enabled: Rc<RefCell<bool>>,
    timeout_found: usize,
    objective_dir: PathBuf,
    triggered: bool,
}

impl CaptureTimeoutFeedback {
    pub fn new(enabled: Rc<RefCell<bool>>, objective_dir: &Path) -> Self {
        Self {
            enabled,
            timeout_found: 0,
            objective_dir: objective_dir.to_path_buf(),
            triggered: false,
        }
    }
}

/// File `testcase` under `prefix` in `dir`.
fn set_filename(dir: &Path, prefix: &str, testcase: &mut Testcase<IrInput>) {
    let base = if let Some(filename) = testcase.filename() {
        filename.clone()
    } else {
        testcase.input().as_ref().unwrap().generate_name(None)
    };
    *testcase.file_path_mut() = Some(dir.join(format!("{prefix}-{base}")));
}

fn fire_count<EM, S>(
    manager: &mut EM,
    state: &mut S,
    name: String,
    count: usize,
) -> Result<(), Error>
where
    S: HasExecutions,
    EM: EventFirer<IrInput, S>,
{
    manager.fire(
        state,
        EventWithStats::with_current_time(
            Event::UpdateUserStats {
                name: Cow::from(name),
                value: UserStats::new(UserStatsValue::Number(count as u64), AggregatorOps::Sum),
                phantom: PhantomData,
            },
            *state.executions(),
        ),
    )
}

impl Named for CaptureTimeoutFeedback {
    fn name(&self) -> &Cow<'static, str> {
        static NAME: Cow<'static, str> = Cow::Borrowed("CaptureTimeoutFeedback");
        &NAME
    }
}

impl<S> StateInitializer<S> for CaptureTimeoutFeedback {}

impl<EM, OT, S> Feedback<EM, IrInput, OT, S> for CaptureTimeoutFeedback
where
    S: HasCorpus<IrInput> + HasMetadata + HasExecutions,
    EM: EventFirer<IrInput, S>,
{
    #[inline]
    fn is_interesting(
        &mut self,
        state: &mut S,
        _manager: &mut EM,
        input: &IrInput,
        _observers: &OT,
        exit_kind: &ExitKind,
    ) -> Result<bool, Error> {
        self.triggered = false;

        if *self.enabled.borrow() && matches!(exit_kind, ExitKind::Timeout) {
            let timeouts = state.metadata_or_insert_with(TimeoutsToVerify::new);
            tracing::info!("timeout detected, queued for verification");
            timeouts.push(input.clone());
            return Ok(false);
        }

        self.triggered = matches!(exit_kind, ExitKind::Timeout);

        Ok(matches!(exit_kind, ExitKind::Timeout))
    }

    fn append_metadata(
        &mut self,
        state: &mut S,
        manager: &mut EM,
        _observers: &OT,
        testcase: &mut Testcase<IrInput>,
    ) -> Result<(), Error> {
        if self.triggered {
            self.timeout_found += 1;
            fire_count(manager, state, "timeout".to_string(), self.timeout_found)?;
            set_filename(&self.objective_dir, "timeout", testcase);
        }

        Ok(())
    }
}

/// Files a finding by what reported it.
///
/// A scenario reports a finding as `FAIL: <oracle>: <detail>` and a run it could not complete
/// as `INFRASTRUCTURE: <detail>`. The oracle's name, without its `Oracle` suffix and in lower
/// case, is the cause: `setupconnection`, `crash`; an infrastructure report is
/// `infrastructure`; anything else on the stream is `other`.
pub struct CrashCauseFeedback {
    handle: Handle<StdOutObserver>,
    counts: HashMap<String, usize>,
    objective_dir: PathBuf,
}

impl CrashCauseFeedback {
    pub fn new(handle: Handle<StdOutObserver>, objective_dir: &Path) -> Self {
        Self {
            handle,
            counts: HashMap::new(),
            objective_dir: objective_dir.to_path_buf(),
        }
    }
}

/// The cause a scenario's output reports, if it reports one.
pub fn cause_of(output: &[u8]) -> Option<String> {
    let re = Regex::new(r"(FAIL|INFRASTRUCTURE): ([A-Za-z]+)").expect("the pattern is valid");
    let caps = re.captures(output)?;
    let kind = caps.get(1)?.as_bytes();
    if kind == b"INFRASTRUCTURE" {
        return Some("infrastructure".to_string());
    }
    let oracle = String::from_utf8_lossy(caps.get(2)?.as_bytes()).to_ascii_lowercase();
    Some(
        oracle
            .strip_suffix("oracle")
            .filter(|s| !s.is_empty())
            .unwrap_or("other")
            .to_string(),
    )
}

impl Named for CrashCauseFeedback {
    fn name(&self) -> &Cow<'static, str> {
        static NAME: Cow<'static, str> = Cow::Borrowed("CrashCauseFeedback");
        &NAME
    }
}

impl<S> StateInitializer<S> for CrashCauseFeedback {}

impl<EM, OT, S> Feedback<EM, IrInput, OT, S> for CrashCauseFeedback
where
    OT: ObserversTuple<IrInput, S>,
    S: HasCorpus<IrInput> + HasMetadata + HasExecutions,
    EM: EventFirer<IrInput, S>,
{
    #[inline]
    fn is_interesting(
        &mut self,
        _state: &mut S,
        _manager: &mut EM,
        _input: &IrInput,
        _observers: &OT,
        _exit_kind: &ExitKind,
    ) -> Result<bool, Error> {
        Ok(false)
    }

    fn append_metadata(
        &mut self,
        state: &mut S,
        manager: &mut EM,
        observers: &OT,
        testcase: &mut Testcase<IrInput>,
    ) -> Result<(), Error> {
        let stdout_observer = observers
            .get(&self.handle)
            .ok_or_else(|| Error::illegal_state("StdOutObserver is missing"))?;
        let Some(cause) = stdout_observer.output.as_deref().and_then(cause_of) else {
            return Ok(());
        };

        *self.counts.entry(cause.clone()).or_insert(0) += 1;
        for (cause, count) in &self.counts {
            fire_count(manager, state, cause.clone(), *count)?;
        }
        set_filename(&self.objective_dir, &cause, testcase);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::cause_of;

    #[test]
    fn a_report_is_filed_by_its_oracle() {
        assert_eq!(
            cause_of(b"FAIL: SetupConnectionOracle: the answer was silence\n").as_deref(),
            Some("setupconnection")
        );
        assert_eq!(
            cause_of(b"[init] ...\nFAIL: CrashOracle: the role stopped serving").as_deref(),
            Some("crash")
        );
        assert_eq!(
            cause_of(b"INFRASTRUCTURE: HarnessIntegrityOracle: action 3 has no outcome").as_deref(),
            Some("infrastructure")
        );
        assert_eq!(
            cause_of(b"FAIL: Oracle: nameless").as_deref(),
            Some("other")
        );
        assert_eq!(cause_of(b"the test case passed"), None);
    }
}
