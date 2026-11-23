# 0067 - Import a detached workspace for cold recovery

A placement authority can recover a workspace on a host that cannot resume the source CPU state.
It identifies a certified destination Generation, tenant, Instance, and volume before the broker requests an import.
The source checkpoint identity and exact workspace digest come from the encrypted backup receipt.
The original placement must be fenced before reusing its Instance identity.

The host volume service opens the fixed workspace artifact on encrypted storage.
It creates a new private backing with a fresh volume key and atomically publishes it as detached.
Existing volume files and current tenant writes are preserved.
A repeated import of the same receipt and destination returns the existing binding.
A subsequent ordinary cold start owns offline repair, attachment, and guest readiness.

The host protocol admits one bounded workspace import frame on its existing privileged local socket.
The import never trusts source host sidecars or key records.

A trusted operator can import a legacy capture for a cold start when its authenticated receipt binds the workspace digest to the source Instance.
The destination receives an explicitly authorized volume ID.
