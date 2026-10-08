#!/bin/sh
set -eu

: "${DATABASE_URL:?DATABASE_URL is required}"
: "${BACKUP_BUCKET:?BACKUP_BUCKET is required}"
: "${BACKUP_ENDPOINT:?BACKUP_ENDPOINT is required}"
: "${AWS_ACCESS_KEY_ID:?AWS_ACCESS_KEY_ID is required}"
: "${AWS_SECRET_ACCESS_KEY:?AWS_SECRET_ACCESS_KEY is required}"
: "${AWS_DEFAULT_REGION:?AWS_DEFAULT_REGION is required}"

backup_prefix=${BACKUP_PREFIX:-backups/postgres}
stamp=$(date -u +%Y%m%d-%H%M%S)
backup_name="interdeck-${stamp}.dump"
backup_file="/tmp/${backup_name}"
checksum_file="${backup_file}.sha256"
trap 'rm -f "$backup_file" "$checksum_file"' EXIT HUP INT TERM

echo "Creating portable PostgreSQL backup ${backup_name}"
pg_dump "$DATABASE_URL" \
  --format=custom \
  --compress=9 \
  --no-owner \
  --no-acl \
  --file="$backup_file"

# A truncated or malformed custom archive must fail before it reaches storage.
pg_restore --list "$backup_file" >/dev/null
(
  cd /tmp
  sha256sum "$backup_name" >"${backup_name}.sha256"
)

destination="s3://${BACKUP_BUCKET}/${backup_prefix}/${stamp}"
aws s3 cp "$backup_file" "${destination}/${backup_name}" \
  --endpoint-url "$BACKUP_ENDPOINT" \
  --only-show-errors
aws s3 cp "$checksum_file" "${destination}/${backup_name}.sha256" \
  --endpoint-url "$BACKUP_ENDPOINT" \
  --only-show-errors

aws s3api head-object \
  --bucket "$BACKUP_BUCKET" \
  --key "${backup_prefix}/${stamp}/${backup_name}" \
  --endpoint-url "$BACKUP_ENDPOINT" >/dev/null
echo "Backup uploaded and verified: ${backup_prefix}/${stamp}/${backup_name}"
