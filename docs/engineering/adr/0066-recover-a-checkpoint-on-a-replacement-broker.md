# 0066 - Recover a checkpoint on a replacement broker

A trusted host-local recovery installer may reserve an absent Instance after the caller has fenced the original host.
The reservation binds the authorized machine name, checkpoint operation, state digest, Generation, and backend.
It refuses to replace an existing machine and exposes no restore owner until the encrypted checkpoint publication is verified.
An identical installer retry can finish an interrupted reservation.
Its input must come from a verified tenant-scoped backup and an authorized placement decision.

Only the named checkpoint may start the reserved Instance.
The same-Instance restore validates the destination Generation and uses the usual encrypted storage, compatibility, cleanup, and first-command checks.
Admission records the reservation as the prior state so a failed or interrupted restore does not convert the Instance into a fresh launch.
A reservation is absent from the running fleet census.
There is no customer-facing endpoint for installing recovery authority.
