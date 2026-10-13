//! Rebuilding the set of stopped volumes after a reboot.
//!
//! The `tmpfs` Instance registry is empty after a reboot and holds only running machines anyway,
//! so the stopped set is rebuilt from the on-disk sidecars plus the write-ahead records.
//! A sidecar whose state is `attached` but whose Instance is not running is recovered to
//! `detached`, because the machine that held it cannot have survived the reboot.
//! A volume whose last write-ahead record is a `begin` with no matching `end` is reported
//! in-flight and is never touched by the reaper, so a crash mid-transaction cannot be mistaken
//! for a clean state.

use crate::{InstanceId, VolumeId};

use super::record::{VolumeRecord, VolumeRecordKind};
use super::sidecar::{VolumeSidecar, VolumeState};

/// One volume that is stopped and carries an owner, ready for the control plane to reconcile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoppedVolume {
    /// The volume.
    pub volume: VolumeId,
    /// The Instance that owns it.
    pub instance: InstanceId,
}

/// What a rebuild found.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RebuildReport {
    /// Every stopped volume, in identity order.
    pub stopped: Vec<StoppedVolume>,
    /// The volumes that were `attached` with no running machine and are now `detached`.
    pub recovered: Vec<VolumeId>,
    /// The volumes already decided destroyed, which the reaper finishes.
    pub deleting: Vec<VolumeId>,
    /// The volumes with an open transaction, which must be repaired and never reaped.
    pub in_flight: Vec<VolumeId>,
}

/// Rebuilds the stopped set from the write-ahead records, the sidecars, and the live Instances.
///
/// `live` is the set of Instances this host currently owns a running machine for, which the
/// caller reads from its own registry rather than from the sidecars, so a sidecar cannot claim a
/// machine is running that the host does not have.
#[must_use]
pub fn rebuild(
    records: &[VolumeRecord],
    sidecars: &[VolumeSidecar],
    live: &[InstanceId],
) -> RebuildReport {
    let mut report = RebuildReport {
        in_flight: open_transactions(records),
        ..RebuildReport::default()
    };
    for sidecar in sidecars {
        match sidecar.state {
            VolumeState::Deleting => report.deleting.push(sidecar.volume),
            VolumeState::Detached => report.stopped.push(StoppedVolume {
                volume: sidecar.volume,
                instance: sidecar.instance,
            }),
            VolumeState::Attached => {
                if !live.contains(&sidecar.instance) {
                    report.recovered.push(sidecar.volume);
                    report.stopped.push(StoppedVolume {
                        volume: sidecar.volume,
                        instance: sidecar.instance,
                    });
                }
            }
        }
    }
    report.stopped.sort_unstable_by_key(|entry| entry.volume);
    report.recovered.sort_unstable();
    report.deleting.sort_unstable();
    report.in_flight.sort_unstable();
    report
}

/// The volumes whose last record is a `begin` with no matching `end`.
fn open_transactions(records: &[VolumeRecord]) -> Vec<VolumeId> {
    let mut open: Vec<(VolumeId, VolumeRecordKind)> = Vec::new();
    for record in records {
        if record.kind.is_begin() {
            if let Some(slot) = open.iter_mut().find(|(volume, _)| *volume == record.volume) {
                slot.1 = record.kind;
            } else {
                open.push((record.volume, record.kind));
            }
        } else if record.kind.is_end() {
            open.retain(|(volume, begin)| {
                !(*volume == record.volume && record.kind.closes(*begin))
            });
        }
    }
    open.into_iter().map(|(volume, _)| volume).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::GenerationId;

    fn instance(byte: u8) -> InstanceId {
        InstanceId::new([byte; 16]).expect("nonzero")
    }

    fn volume(byte: u8) -> VolumeId {
        VolumeId::new([byte; 16]).expect("nonzero")
    }

    fn sidecar(volume_byte: u8, instance_byte: u8, state: VolumeState) -> VolumeSidecar {
        VolumeSidecar {
            volume: volume(volume_byte),
            instance: instance(instance_byte),
            generation: GenerationId::new([1; 32]).expect("nonzero"),
            tenant: "tenant-alpha".to_owned(),
            size_class_gib: 10,
            state,
            dek_id: [0; 16],
            created_at_nanos: 1,
            last_transition_nanos: 2,
        }
    }

    #[test]
    fn a_detached_sidecar_is_stopped_and_not_recovered() {
        let report = rebuild(&[], &[sidecar(1, 9, VolumeState::Detached)], &[]);
        assert_eq!(
            report.stopped,
            vec![StoppedVolume {
                volume: volume(1),
                instance: instance(9)
            }]
        );
        assert!(report.recovered.is_empty());
        assert!(report.deleting.is_empty());
    }

    #[test]
    fn an_attached_sidecar_with_no_running_machine_is_recovered_to_stopped() {
        let report = rebuild(&[], &[sidecar(1, 9, VolumeState::Attached)], &[]);
        assert_eq!(report.recovered, vec![volume(1)]);
        assert_eq!(report.stopped.len(), 1);
    }

    #[test]
    fn an_attached_sidecar_with_a_running_machine_is_left_alone() {
        let report = rebuild(&[], &[sidecar(1, 9, VolumeState::Attached)], &[instance(9)]);
        assert!(report.stopped.is_empty());
        assert!(report.recovered.is_empty());
    }

    #[test]
    fn a_deleting_sidecar_is_never_stopped() {
        let report = rebuild(&[], &[sidecar(1, 9, VolumeState::Deleting)], &[]);
        assert_eq!(report.deleting, vec![volume(1)]);
        assert!(report.stopped.is_empty());
    }

    #[test]
    fn stopped_volumes_are_reported_in_identity_order() {
        let sidecars = [
            sidecar(3, 9, VolumeState::Detached),
            sidecar(1, 8, VolumeState::Detached),
            sidecar(2, 7, VolumeState::Detached),
        ];
        let report = rebuild(&[], &sidecars, &[]);
        let order: Vec<_> = report.stopped.iter().map(|entry| entry.volume).collect();
        assert_eq!(order, vec![volume(1), volume(2), volume(3)]);
    }

    #[test]
    fn a_matched_begin_and_end_is_not_in_flight() {
        let records = [
            VolumeRecord::new(VolumeRecordKind::AttachBegin, volume(1)),
            VolumeRecord::new(VolumeRecordKind::AttachEnd, volume(1)),
        ];
        assert!(rebuild(&records, &[], &[]).in_flight.is_empty());
    }

    #[test]
    fn an_unmatched_begin_is_in_flight() {
        let records = [VolumeRecord::new(VolumeRecordKind::DetachBegin, volume(1))];
        assert_eq!(rebuild(&records, &[], &[]).in_flight, vec![volume(1)]);
    }

    #[test]
    fn an_end_that_does_not_close_the_begin_leaves_it_in_flight() {
        // A create that was closed by an attach end is a state machine violation, and the volume
        // stays in-flight rather than being read as a clean create.
        let records = [
            VolumeRecord::new(VolumeRecordKind::CreateBegin, volume(1)),
            VolumeRecord::new(VolumeRecordKind::AttachEnd, volume(1)),
        ];
        assert_eq!(rebuild(&records, &[], &[]).in_flight, vec![volume(1)]);
    }

    #[test]
    fn a_later_begin_replaces_an_earlier_open_one() {
        let records = [
            VolumeRecord::new(VolumeRecordKind::AttachBegin, volume(1)),
            VolumeRecord::new(VolumeRecordKind::DetachBegin, volume(1)),
            VolumeRecord::new(VolumeRecordKind::DetachEnd, volume(1)),
        ];
        assert!(rebuild(&records, &[], &[]).in_flight.is_empty());
    }

    #[test]
    fn an_empty_host_rebuilds_to_nothing() {
        assert_eq!(rebuild(&[], &[], &[]), RebuildReport::default());
    }
}
