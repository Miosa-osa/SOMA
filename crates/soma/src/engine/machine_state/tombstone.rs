use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::{
    ExecutionReceipt, OperationId, OperationKind, RequestFingerprint, StateStoreFailure,
    TerminalStatus,
};

use super::{ExecutionTombstone, corrupt};

impl ExecutionTombstone {
    pub(in crate::engine) fn from_receipt(
        receipt: &ExecutionReceipt,
    ) -> Result<Self, StateStoreFailure> {
        if receipt.operation() != OperationKind::Execute
            || !matches!(
                receipt.terminal_status(),
                TerminalStatus::Exited { .. } | TerminalStatus::Signaled { .. }
            )
        {
            return Err(corrupt());
        }
        let encoded = serde_json::to_vec(receipt).map_err(|_| corrupt())?;
        Ok(Self {
            operation_id: receipt.operation_id().clone(),
            request_fingerprint: receipt.request_fingerprint().clone(),
            terminal_status: *receipt.terminal_status(),
            receipt_digest: crate::fingerprint::digest_bytes(&encoded),
        })
    }

    fn encode(&self) -> String {
        let status = match self.terminal_status {
            TerminalStatus::Exited { code } => format!("e{:08x}", code.cast_unsigned()),
            TerminalStatus::Signaled { signal: None } => "sn".to_owned(),
            TerminalStatus::Signaled {
                signal: Some(signal),
            } => format!("s{:08x}", signal.cast_unsigned()),
            _ => unreachable!("validated tombstones contain only terminal command outcomes"),
        };
        format!(
            "v1:{}:{}:{}:{status}",
            self.operation_id.as_str(),
            digest_hex(&self.request_fingerprint),
            digest_hex(&self.receipt_digest),
        )
    }

    fn decode(value: &str) -> Result<Self, StateStoreFailure> {
        let mut fields = value.split(':');
        if fields.next() != Some("v1") {
            return Err(corrupt());
        }
        let operation_id =
            OperationId::new(fields.next().ok_or_else(corrupt)?).map_err(|_| corrupt())?;
        let request_fingerprint = parse_digest_hex(fields.next().ok_or_else(corrupt)?)?;
        let receipt_digest = parse_digest_hex(fields.next().ok_or_else(corrupt)?)?;
        let terminal_status = parse_terminal(fields.next().ok_or_else(corrupt)?)?;
        if fields.next().is_some() {
            return Err(corrupt());
        }
        Ok(Self {
            operation_id,
            request_fingerprint,
            terminal_status,
            receipt_digest,
        })
    }
}

impl Serialize for ExecutionTombstone {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.encode())
    }
}

impl<'de> Deserialize<'de> for ExecutionTombstone {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::decode(&String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

fn digest_hex(value: &RequestFingerprint) -> &str {
    value
        .as_str()
        .strip_prefix("sha256:")
        .expect("validated request fingerprints have the sha256 prefix")
}

fn parse_digest_hex(value: &str) -> Result<RequestFingerprint, StateStoreFailure> {
    RequestFingerprint::new(format!("sha256:{value}")).map_err(|_| corrupt())
}

fn parse_terminal(value: &str) -> Result<TerminalStatus, StateStoreFailure> {
    if value == "sn" {
        return Ok(TerminalStatus::Signaled { signal: None });
    }
    let (kind, encoded) = value.split_at_checked(1).ok_or_else(corrupt)?;
    if encoded.len() != 8 {
        return Err(corrupt());
    }
    let number = u32::from_str_radix(encoded, 16).map_err(|_| corrupt())?;
    match kind {
        "e" => Ok(TerminalStatus::Exited {
            code: number.cast_signed(),
        }),
        "s" => Ok(TerminalStatus::Signaled {
            signal: Some(number.cast_signed()),
        }),
        _ => Err(corrupt()),
    }
}
