//! What a starting broker does with what the previous one left behind.
//!
//! A previous process leaves two things the kernel keeps: a prepared pool no ledger record owns,
//! and the ruleset of any bundle whose egress window it had opened. Both are settled here, and a
//! failure to settle either is reported and never stops the broker from serving.

use crate::{
    Broker,
    reconcile::{MAX_REAP_PER_START, ReapPlan, reap_plan},
    recover::restore_base_rulesets,
};

/// Decides which unassigned bundles to reap, before this process builds any of its own.
///
/// The pool a previous process prepared has no ledger owner. What to remove is decided now,
/// before this process builds any bundle, so removal can run beside the listener without being
/// able to name one of its own. A failure to decide must not stop egress brokering, so it is
/// reported and the daemon starts anyway.
pub(super) fn plan_reap(broker: &Broker) -> ReapPlan {
    match reap_plan(broker) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("soma-netd: start could not plan a reap of unassigned bundles: {error}");
            ReapPlan::default()
        }
    }
}

/// Puts every bundle claimed with no egress back to its base ruleset.
///
/// A window that was open when the previous broker ended has no timer and no record now, and the
/// kernel kept its ruleset. Every bundle claimed with no egress is put back to that base before
/// the first request is served, so a restart can only close, never leave open.
pub(super) fn restore_bases(broker: &Broker) {
    match restore_base_rulesets(broker) {
        Ok(report) => eprintln!(
            "soma-netd: start restored {} denied bundle(s) to their base; forwarding switched off for [{}]; could not restore [{}]",
            report.restored.len(),
            report.forwarding_off.join(","),
            report.failed.join(",")
        ),
        Err(error) => eprintln!(
            "soma-netd: WARN: start could not read the ledger to restore denied bundles: {error}"
        ),
    }
}

/// Removes what `plan` named, up to the per-start limit, and reports what happened.
pub(super) fn reap(broker: &Broker, plan: ReapPlan) {
    let report = plan.execute(broker, MAX_REAP_PER_START);
    eprintln!(
        "soma-netd: start reaped {} unassigned bundle(s); in use [{}]; failed [{}]; deferred {}",
        report.removed.len(),
        report.in_use.join(","),
        report.failed.join(","),
        report.deferred.len()
    );
    if !report.foreign.is_empty() {
        eprintln!(
            "soma-netd: WARN: kernel objects with the broker's naming and no pin of ours were left alone (another broker, or a bundle whose pin is gone): [{}]",
            report.foreign.join(",")
        );
    }
    if !report.failed.is_empty() {
        eprintln!(
            "soma-netd: WARN: could not fully remove [{}]; they keep their pin and are retried at the next start",
            report.failed.join(",")
        );
    }
}
