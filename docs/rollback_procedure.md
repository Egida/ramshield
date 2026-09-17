# Staging Rollback Procedure

Status: staging procedure. Not a production readiness claim.

Purpose: stop a RamShield pilot slice safely, preserve evidence, restore the prior edge policy, and verify that the protected service remains reachable.

## Preconditions

- Record commit/image digest, config hash, interface, traffic slice, active policy, WAL path, and operator.
- Confirm the customer's existing edge policy and rollback owner.
- Test this procedure in staging before any production exposure.
- Keep the prior edge configuration available and validated.

## Stop conditions

Rollback immediately on unmeasured shared-IP collateral, health failure, queue growth, WAL error, unexpected XDP state, or any behavior outside the written pilot scope. Do not wait for a benchmark target.

## Host / systemd staging

```text
1. Stop new test traffic.
2. Restore the previously saved edge/proxy policy.
3. Stop RamShield:
   sudo systemctl stop ramshield
4. Verify the service stopped:
   systemctl is-active ramshield
5. Verify the interface and edge policy no longer use the pilot path.
6. Check service logs and dashboard health for the rollback window.
7. Preserve the WAL and logs; do not delete them before evidence capture.
8. Re-run the agreed health and reachability checks.
```

Expected result: the prior edge policy handles the traffic slice; RamShield is inactive; logs and WAL remain available for diagnosis.

## Kubernetes staging

```text
1. Stop new test traffic.
2. Restore the previously saved edge/proxy manifest or policy.
3. Scale the pilot DaemonSet down:
   kubectl -n ramshield scale daemonset/ramshield-node-guard --replicas=0
4. Verify no pilot pod remains ready:
   kubectl -n ramshield get pods -l app.kubernetes.io/name=ramshield
5. Verify the edge path and service health.
6. Preserve pod logs and the WAL hostPath before cleanup.
7. Re-run the agreed health and reachability checks.
```

If the deployment controller recreates pods, remove or suspend the staging DaemonSet through the approved change path; do not improvise deletion during an incident.

## Evidence capture

Record UTC start/end, operator, reason, traffic slice, policy before/after, commands, service/pod status, health results, XDP status, WAL errors, shared-IP observations, and unresolved uncertainty. Redact credentials, customer identifiers, IP traces, and internal topology before sharing.

## Recovery review

A rollback is not a success metric. Review whether the prior policy restored service, whether any rules remained active, whether WAL replay is needed, and whether the pilot scope or safety gate must change before another run.

Related evidence: `scripts/final_integration.py`, `scripts/prod_smoke.sh`, `docs/PRODUCTION_READINESS.md`, and the customer-specific pilot scope.
