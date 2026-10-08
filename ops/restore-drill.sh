#!/bin/sh
set -eu

: "${BACKUP_FILE:?BACKUP_FILE must point to a downloaded custom-format dump}"
: "${RESTORE_DATABASE_URL:?RESTORE_DATABASE_URL must point to a disposable scratch database}"
: "${ALLOW_RESTORE_DRILL:?Set ALLOW_RESTORE_DRILL=YES after checking the target}"

if [ "$ALLOW_RESTORE_DRILL" != "YES" ]; then
  echo "Refusing restore drill without ALLOW_RESTORE_DRILL=YES" >&2
  exit 2
fi
if [ ! -f "$BACKUP_FILE" ]; then
  echo "Backup file does not exist: $BACKUP_FILE" >&2
  exit 2
fi

url_without_query=${RESTORE_DATABASE_URL%%\?*}
query=${RESTORE_DATABASE_URL#*\?}
if [ "$query" = "$RESTORE_DATABASE_URL" ]; then query=""; fi
drill_database=${url_without_query##*/}
connection_prefix=${url_without_query%/*}
maintenance_url="${connection_prefix}/postgres"
if [ -n "$query" ]; then maintenance_url="${maintenance_url}?${query}"; fi

case "$drill_database" in
  interdeck_restore_drill_*) ;;
  *)
    echo "Refusing target database '$drill_database'; its name must start with interdeck_restore_drill_" >&2
    exit 2
    ;;
esac

cleanup() {
  if [ "${KEEP_RESTORE_DRILL:-0}" != "1" ]; then
    dropdb --if-exists --force --maintenance-db="$maintenance_url" "$drill_database"
  fi
}
trap cleanup EXIT HUP INT TERM

pg_restore --list "$BACKUP_FILE" >/dev/null
dropdb --if-exists --force --maintenance-db="$maintenance_url" "$drill_database"
createdb --maintenance-db="$maintenance_url" "$drill_database"

started_at=$(date +%s)
pg_restore \
  --dbname="$RESTORE_DATABASE_URL" \
  --no-owner \
  --no-acl \
  --exit-on-error \
  "$BACKUP_FILE"

psql "$RESTORE_DATABASE_URL" -v ON_ERROR_STOP=1 -c \
  "SELECT 'decks' AS relation, count(*) FROM decks
   UNION ALL SELECT 'deck_revisions', count(*) FROM deck_revisions
   UNION ALL SELECT 'response_events', count(*) FROM response_events
   UNION ALL SELECT 'questions', count(*) FROM questions
   ORDER BY relation;"

elapsed=$(( $(date +%s) - started_at ))
echo "Restore drill passed in ${elapsed}s using database ${drill_database}"
if [ "${KEEP_RESTORE_DRILL:-0}" = "1" ]; then
  echo "Scratch database retained because KEEP_RESTORE_DRILL=1"
fi
