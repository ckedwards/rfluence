#!/usr/bin/env bash
# Capture real Confluence responses for the reference pages as test fixtures.
#
# For each page, writes fixtures/confluence/<id>/:
#   adf.json          the page body (ADF), decoded and pretty-printed
#   page.json         the rest of the v2 page response (title, version, labels, links)
#   attachments.json  the page's attachments (only for pages that have any)
#
# Usage: scripts/capture-fixtures.sh [page id ...]
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

# Reference pages from design.md ("Useful info").
default_pages=(
  295341 # merfluence: diagram inserted in the editor (cached SVGs, embeddedMacroContext)
  458755 # merfluence API test: minimal / default-settings / stale-SVG diagrams
  131074 # image API test: minimal node, explicit size, new attachment version, external image
  295349 # link API test: page links, smart link, headings for anchor IDs
  295257 # human-edited page with an editor-inserted image and smart links
  426008 # emoji API test: shortName only, all attrs, GitHub alias, unknown name, ZWJ
  458790 # ADF reference: every common node type, API-created plus an editor-added section
  98404  # code languages (197 names) and breakout / table width variants
  524309 # label API test
)
pages=("${@:-${default_pages[@]}}")

get() {
  curl -sSf -u "$CONFLUENCE_EMAIL:$CONFLUENCE_API_KEY" -H 'Accept: application/json' \
    "${CONFLUENCE_BASE_URL%/}/wiki/api/v2/$1"
}

for id in "${pages[@]}"; do
  dir="$out/$id"
  mkdir -p "$dir"
  page="$(get "pages/$id?body-format=atlas_doc_format&include-labels=true")"
  jq '.body.atlas_doc_format.value | fromjson' <<<"$page" >"$dir/adf.json"
  jq 'del(.body)' <<<"$page" >"$dir/page.json"

  attachments="$(get "pages/$id/attachments?limit=250")"
  if [[ "$(jq '.results | length' <<<"$attachments")" -gt 0 ]]; then
    jq '.' <<<"$attachments" >"$dir/attachments.json"
  else
    rm -f "$dir/attachments.json"
  fi

  echo "$id  $(jq -r .title "$dir/page.json")  ($(wc -c <"$dir/adf.json") bytes ADF)"
done
