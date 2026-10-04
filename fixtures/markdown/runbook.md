# Runbook: Database Failover

> [!IMPORTANT]
> Only on-call engineers with production access should run this procedure.

## Before you start

- [ ] Page the database owner in `#db-oncall`
- [ ] Confirm the replica lag is under 5 seconds
- [ ] Open a change ticket

## Procedure

1. Check the replica status:

   ```bash
   psql -h replica-1.internal -c "SELECT now() - pg_last_xact_replay_timestamp() AS lag;"
   ```

2. Stop writes to the primary:
   1. Set the app to read-only mode with <kbd>Ctrl</kbd>+<kbd>R</kbd> in the admin console.
   2. Wait for in-flight transactions to finish.
3. Promote the replica:

   ```bash
   pg_ctl promote -D /var/lib/postgresql/data
   ```

   > [!TIP]
   > If `pg_ctl` isn't on the `PATH`, use the full path: `/usr/lib/postgresql/16/bin/pg_ctl`.

4. Update the DNS record `db.internal` to point at the new primary.

> [!CAUTION]
> Do **not** restart the old primary until it has been rebuilt as a replica. Two primaries will cause split-brain.

## Verifying

Run the health check[^health] and confirm all checks pass ✅. If anything fails, roll back immediately :rotating_light:.

<details>
<summary>Expected health check output</summary>

```text
db.primary ........ ok
db.replica ........ ok
replication lag ... 0.2s
```

</details>

## Rollback

* Point `db.internal` back at the old primary.
* Re-enable writes.
  + Clear the read-only flag.
  + Notify the team.

Contact the DB team if you are unsure.[^team]

[^health]: The health check lives at `scripts/health.sh` and takes about 30 seconds.
[^team]: Slack `#db-team`, or page via PagerDuty.
