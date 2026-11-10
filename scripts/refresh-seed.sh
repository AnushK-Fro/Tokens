#!/usr/bin/env bash
# Regenerates seed/tokens.sql from the database in .env's DATABASE_URL (schema, triggers, the
# sqlx migration record and every row). docker compose loads it into an empty MariaDB volume on
# first start; see README "Deploying with Docker".
set -euo pipefail
cd "$(dirname "$0")/.."

DATABASE_URL="${DATABASE_URL:-$(grep -E '^DATABASE_URL=' .env | tail -1 | cut -d= -f2-)}"
read -r user password host port database < <(python3 -I -c '
import sys, urllib.parse as u
p = u.urlparse(sys.argv[1])
print(u.unquote(p.username or ""), u.unquote(p.password or ""), p.hostname or "localhost", p.port or 3306, p.path.lstrip("/"))
' "$DATABASE_URL")
# localhost would mean the Unix socket to the client; the URL means TCP.
[ "$host" = localhost ] && host=127.0.0.1

mkdir -p seed
{
    echo "-- Tokens seed: schema, triggers and data, dumped $(date -u +%Y-%m-%dT%H:%MZ) by scripts/refresh-seed.sh."
    echo "-- Loaded by MariaDB's /docker-entrypoint-initdb.d on the first start with an empty volume."
    # DEFINER clauses name the dumping user@host, which doesn't exist in the container; without
    # them the triggers belong to whoever loads the seed.
    MYSQL_PWD="$password" mariadb-dump --host="$host" --port="$port" --user="$user" \
        --single-transaction --triggers --hex-blob --skip-dump-date --skip-comments "$database" \
        | sed -E 's#/\*!50017 DEFINER=`[^`]*`@`[^`]*`\*/ ?##g; s# DEFINER=`[^`]*`@`[^`]*`##g'
} > seed/tokens.sql.tmp
mv seed/tokens.sql.tmp seed/tokens.sql

echo "wrote seed/tokens.sql ($(wc -c < seed/tokens.sql) bytes)"
grep -cE '^INSERT INTO' seed/tokens.sql | xargs echo "insert statements:"
