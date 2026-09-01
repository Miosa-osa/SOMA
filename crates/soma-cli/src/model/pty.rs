//! What the envelope says about one terminal operation.

use serde::Serialize;
use soma::{InstanceId, PtyAnswer, PtyRefusal};

use super::OutputBytes;

/// One completed terminal operation.
///
/// Every member a given operation does not answer with is absent from the document rather than
/// present and empty, so a reader cannot mistake "this operation says nothing about it" for
/// "this operation says it is empty".
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct PtyReport {
    pub instance_id: InstanceId,
    pub operation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputBytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<bool>,
}

impl PtyReport {
    /// Builds the report for one answer.
    #[must_use]
    pub fn new(instance_id: InstanceId, operation: &'static str, answer: &PtyAnswer) -> Self {
        let mut report = Self {
            instance_id,
            operation,
            refusal: None,
            columns: None,
            rows: None,
            written: None,
            output: None,
            ended: None,
        };
        match answer {
            PtyAnswer::Opened { columns, rows } | PtyAnswer::Resized { columns, rows } => {
                report.columns = Some(*columns);
                report.rows = Some(*rows);
            }
            PtyAnswer::Wrote { bytes } => report.written = Some(*bytes),
            PtyAnswer::Output { bytes, end } => {
                report.output = Some(OutputBytes::new(bytes));
                report.ended = Some(*end);
            }
            PtyAnswer::Closed => {}
            PtyAnswer::Refused(refusal) => report.refusal = Some(PtyRefusal::code(*refusal)),
        }
        report
    }

    /// Whether the guest declined the operation.
    #[must_use]
    pub const fn refused(&self) -> bool {
        self.refusal.is_some()
    }
}
