# Troubleshooting

Errors go to stderr and say what to do. Every command exits with one of these codes:

| Exit | Meaning |
| --- | --- |
| 0 | Done |
| 1 | Content Confluence can't store (`rfluence check` errors, or warnings with `--warnings-are-errors`) |
| 2 | A usage problem: bad arguments, not logged in, no site, a link to a file without a page, a missing image, a bad config |
| 3 | Not found: no such page (or it's in the trash), or no such `--section` |
| 4 | The token wasn't accepted, or your account isn't allowed to do this |
| 5 | Confluence or the network failed (after retries) |
| 6 | Refused so nothing is lost: the page changed in Confluence since you fetched it, a page with that title already exists, or `fetch -o` would overwrite your local changes |

## Common problems

**"not logged in"** or **"which Confluence site?"**: run `rfluence auth login`, and `rfluence config set default-site <site>` (see [Authentication](authentication.md)).

**"Confluence didn't accept the API token"**: the token expired, was revoked, or was mistyped. Create a new one and run `rfluence auth login`. Confluence treats a bad token as an anonymous visitor, so this can also show up first as a page that seems to be missing; rfluence checks and tells you.

**"permission denied"**: the login works, but your account can't do this on this page or space. Ask a space admin.

**"the page has changed in Confluence since this file was fetched"** (exit 6): someone edited the page after you fetched it. Fetch it to another file, carry your edits over, and upload that. `--force` overwrites their changes.

**"Confluence is busy (HTTP 429); trying again in 2 s"**: Confluence is rate-limiting; rfluence waits and retries. Not an error unless it gives up.

**An upload stopped part-way**: run the same command again. New pages' IDs are written into their files as soon as the pages exist, so nothing is created twice.

**Something else looks wrong with Confluence itself**: check <https://status.atlassian.com>.
