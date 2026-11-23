# Management-plane security

The management plane controls access to instance lifecycle operations, consoles, tunnels, and host administration.
A zero-trust deployment evaluates access to each protected resource using authenticated identity and policy.
Network location alone grants no access. [NIST SP 800-207](https://csrc.nist.gov/pubs/sp/800/207/final)

An agent inside a SOMA instance runs with its guest permissions.
The platform applies management policy when a person, service, or agent requests a management operation.
For example, an agent writing a file in its guest workspace uses the guest's filesystem permissions.
A client opening that instance's desktop tunnel goes through the platform's authorization boundary.

## SOMA interfaces today

| Interface | Enforcement |
| --- | --- |
| Platform runner `/api/v1` | The runner checks the bearer key and tenant policy supplied by the control plane before dispatch. |
| Host API `/v1` | The service validates the format of `x-soma-tenant` and uses it as tenant context. A trusted caller or gateway supplies that identity after authenticating and authorizing the request. |
| Local stdio MCP | The invoking process and its OS permissions define the access boundary. |
| Host-to-guest control | The instance uses fresh guest authority and authenticates readiness to the host as part of its isolation and launch protocol. |

See the [runner admission code](../../crates/soma-api/src/runner/keys.rs), [host API identity handling](../../crates/soma-api/src/tenant.rs), [API reference](../product/reference/api-surface.md), and [threat model](threat-model.md).

## Responsibilities across the five pillars

The [CISA Zero Trust Maturity Model](https://www.cisa.gov/sites/default/files/2023-04/zero_trust_maturity_model_v2_508.pdf) is an assessment framework for the deploying organization.
This mapping identifies which controls the organization evaluates when using SOMA.

| Pillar | What the platform or operator governs |
| --- | --- |
| Identity | The identities of operators and services, their permissions for each resource, and credential issuance and revocation. |
| Devices | The posture of devices that can reach administrative surfaces and the enrollment of hosts that receive management traffic. |
| Networks | Authorized management and host-to-host paths, with protected transport and guest network policy. |
| Applications and workloads | Which principals can launch a workload, operate an instance, or open a tunnel. SOMA supplies the guest isolation boundary. |
| Data | Who can access instance files, volumes, snapshots, exports, and credentials, and what retention or recovery policies apply. |

The organization also owns visibility and analytics, automation and orchestration, and governance across those pillars.
Runtime telemetry can inform the platform's decisions, while the gateway or service that serves the operation enforces them.
