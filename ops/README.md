# Operating Interdeck

## Deploy and upgrade

Use the single-host Compose deployment in the root README, or run the gateway/API containers with private PostgreSQL and S3-compatible storage on your own provider. Expose only the gateway through HTTPS. Keep database, Google, AI, and storage credentials on the API; the gateway needs only its internal and Slidev secrets. Multiple API replicas require shared object storage.

Back up PostgreSQL and asset storage before an upgrade. Deploy a tested revision outside live presentations. API startup applies embedded forward migrations; rolling back an image does not undo a database migration. Test upgrades and restores in a disposable staging installation before changing a live one. Preserve the source database during recovery and cut over only after verifying a restored copy.

## Backups and retention

For managed PostgreSQL, enable and verify the provider's snapshots and point-in-time recovery if available. In addition, `Dockerfile.backup` runs a portable logical dump as a one-shot container; schedule it with your host/provider cron (for example daily at 03:00 UTC). It requires:

- `DATABASE_URL`, using the private database connection.
- `BACKUP_BUCKET`, `BACKUP_ENDPOINT`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and `AWS_DEFAULT_REGION` for a private S3-compatible backup bucket.
- Optional `BACKUP_PREFIX`, default `backups/postgres`.

Use a restricted environment file or secret store; do not place credentials in committed Compose files or shell history. Each run creates a compressed custom-format dump, verifies its catalog, uploads the dump and SHA-256 sidecar, confirms the object exists, and exits. The backup image does not silently delete old artifacts. Configure an explicit bucket lifecycle retention period and access/encryption policy for your installation. Audience record retention in the app does not erase backups or logs.

Database dumps do not contain uploaded assets. Back up the local `assets` volume or enable object versioning/backups on your S3 bucket. Coordinate database and asset recovery; verify all asset references after a restore. Restrict backup access as strictly as the live data.

## Restore drill

At least quarterly and after changing backup infrastructure, download a dump and its checksum, verify it, and restore into a scratch database whose name begins with `interdeck_restore_drill_`:

```sh
sha256sum -c interdeck-YYYYMMDD-HHMMSS.dump.sha256
BACKUP_FILE=./interdeck-YYYYMMDD-HHMMSS.dump \
RESTORE_DATABASE_URL='postgresql://USER:PASSWORD@HOST/interdeck_restore_drill_example' \
ALLOW_RESTORE_DRILL=YES \
./ops/restore-drill.sh
```

Use a private environment file for real credentials. The script recreates only the named scratch database, restores with `--exit-on-error`, prints core row counts, and removes that scratch database afterwards. `KEEP_RESTORE_DRILL=1` retains it for deeper inspection. Record the dump age, duration, row-count comparison, asset checks, operator, and date outside the public repository. A successful upload alone is not a verified backup.

## Health and alerts

- Gateway liveness: `/_gateway/health`. API liveness/readiness: `/health/live` and `/health/ready` on the private API network. Container health checks call these endpoints.
- Private `/internal/metrics` requires `x-interdeck-internal-token`; never proxy `/internal` publicly. Monitor PostgreSQL listener readiness, active SSE connections, snapshot failures, pool pressure, AI retries, HTTP failures, and latency.
- Alert on unhealthy containers, sustained request failures, a stale/missing scheduled backup, failed restore drills, disk/object-store errors, and unusual provider usage. Choose thresholds from measurements of your installation; the app does not provision an external monitoring service.
- Maintenance retries failed object deletion every five minutes. Monitor `asset_gc` queue age and attempts in the private database. Optional audience retention runs every minute and skips decks with a live presentation.

## Presentation and incident procedure

Before an event, check health/listener readiness, creator login, asset loading, current-version presenter preflight, and a rehearsal from an audience phone. Measure isolated event-scale traffic with `load:audience`; include reconnect storms and simultaneous decks. Freeze deployments, migrations, and database/storage configuration during the event.

During a state-changing incident, freeze audience input through presenter controls if available. Preserve the source database and private logs, recover to a sibling installation, verify it, then make an explicit cutover decision. Rotate exposed credentials. Keep incident records and real audience information outside Git.

Provider-specific backup controls vary. Railway users can consult the official [PostgreSQL backup guide](https://docs.railway.com/guides/postgres-backups-restores), [PITR guide](https://docs.railway.com/volumes/point-in-time-recovery), and [cron guide](https://docs.railway.com/cron-jobs). Verify the controls available on your plan before relying on them.
