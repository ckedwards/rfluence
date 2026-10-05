#!/usr/bin/env bash
# Capture Confluence pages as test fixtures in fixtures/confluence/<name>/.
# See fixtures/confluence/README.md.
#
# Usage:
#   scripts/capture-fixtures.sh <name>...               re-capture these fixtures
#   scripts/capture-fixtures.sh --all                   re-capture every fixture
#   scripts/capture-fixtures.sh --new <name> <page id>  capture a new page
#   --force   also overwrite fixtures that have saved earlier versions (adf.v*.json)
#
# Each fixture's page ID is read from its page.json. Writes:
#   adf.json          the page body (ADF), decoded and pretty-printed
#   page.json         the rest of the v2 page response (id, title, version, labels, links)
#   attachments.json  the page's attachments (pages that have any)
#   attachments/      the attachment files, so the page can be recreated elsewhere
#
# Credentials come from the environment, or from .env in the repo root.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/fixtures/confluence"

if [[ -z "${CONFLUENCE_API_KEY:-}${CONFLUENCE_EMAIL:-}${CONFLUENCE_BASE_URL:-}" && -f "$root/.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  . "$root/.env"
  set +a
fi
for var in CONFLUENCE_API_KEY CONFLUENCE_EMAIL CONFLUENCE_BASE_URL; do
  [[ -n "${!var:-}" ]] || { echo "error: $var is not set" >&2; exit 1; }
done
base="${CONFLUENCE_BASE_URL%/}"

usage() { sed -n '5,9p' "$0" | sed 's/^# //' >&2; exit 2; }

force=false
names=()
declare -A ids=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --force) force=true ;;
    --all) for d in "$out"/*/; do names+=("$(basename "$d")"); done ;;
    --new)
      [[ $# -ge 3 ]] || usage
      names+=("$2"); ids[$2]="$3"; shift 2 ;;
    -*) echo "unknown option $1" >&2; usage ;;
    *) names+=("$1") ;;
  esac
  shift
done
[[ ${#names[@]} -gt 0 ]] || usage

get() {
  curl -sSf -u "$CONFLUENCE_EMAIL:$CONFLUENCE_API_KEY" -H 'Accept: application/json' "$base/wiki/api/v2/$1"
}

for name in "${names[@]}"; do
  [[ "$name" =~ ^[a-z0-9][a-z0-9-]*$ ]] || { echo "error: fixture names are lowercase-with-dashes: $name" >&2; exit 2; }
  dir="$out/$name"
  id="${ids[$name]:-}"
  if [[ -z "$id" ]]; then
    [[ -f "$dir/page.json" ]] || { echo "error: no fixture $name (use --new $name <page id>)" >&2; exit 1; }
    id="$(jq -r .id "$dir/page.json")"
  fi
  if compgen -G "$dir/adf.v*.json" >/dev/null && ! $force; then
    echo "skipping $name: it has saved earlier versions used by the editor-save tests (--force to overwrite)" >&2
    continue
  fi
  page="$(get "pages/$id?body-format=atlas_doc_format&include-labels=true")"
  attachments="$(get "pages/$id/attachments?limit=250")"
  mkdir -p "$dir"
  jq '.body.atlas_doc_format.value | fromjson' <<<"$page" >"$dir/adf.json"
  # Sorted: Confluence returns keys in a different order on each request.
  jq -S 'del(.body)' <<<"$page" >"$dir/page.json"

  rm -rf "$dir/attachments"
  if [[ "$(jq '.results | length' <<<"$attachments")" -gt 0 ]]; then
    jq -S '.' <<<"$attachments" >"$dir/attachments.json"
    mkdir -p "$dir/attachments"
    while IFS=$'\t' read -r title link; do
      curl -sSfL -u "$CONFLUENCE_EMAIL:$CONFLUENCE_API_KEY" "$base/wiki$link" -o "$dir/attachments/$title"
    done < <(jq -r '.results[] | [.title, ._links.download] | @tsv' <<<"$attachments")
  else
    rm -f "$dir/attachments.json"
  fi

  echo "$name ($id): $(jq -r .title "$dir/page.json"), $(wc -c <"$dir/adf.json") bytes ADF, $(find "$dir/attachments" -type f 2>/dev/null | wc -l) attachment files"
done
