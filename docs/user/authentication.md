# Authentication

rfluence works with Confluence Cloud and logs in with your Atlassian account's email and an API token. One token works on every Confluence site your account belongs to.

## Create an API token

1. Go to <https://id.atlassian.com/manage-profile/security/api-tokens>.
2. Choose **Create API token** (not "with scopes": scoped tokens aren't supported yet), name it (e.g. `rfluence`), and pick an expiry date (up to a year).
3. Copy the token.

## Log in

```shell
rfluence auth login
```

It asks for:

- your **Confluence site**, the first time: `example` (for `example.atlassian.net`) or the full host. It becomes your default site;
- your **email** (it suggests your git email);
- the **token** (it isn't shown as you type).

The token is checked against Confluence before it's saved; a mistyped one is asked for again. It's stored in your system keyring (GNOME Keyring or KWallet on Linux, Keychain on macOS, Credential Manager on Windows), or, where there's none, in a file only you can read.

## The default site

Commands that name a page by its URL use that page's site. Everything else (page IDs, search, new pages) uses the default site:

```shell
rfluence config set default-site example   # or example.atlassian.net
rfluence config get default-site
```

`--site <site>` on a command, or the `RFLUENCE_SITE` environment variable, uses another site for one command.

## Check and manage the login

```shell
rfluence auth status    # your email, where the token is, and whether it works on the default site
rfluence auth login     # again, to replace an expired token
rfluence auth logout    # forget the email and token
```

Tokens expire (at most a year after they're created). When yours does, commands say "Confluence didn't accept the API token…": create a new one and run `rfluence auth login`.

## CI and scripts

Set all three environment variables; they're used for their own site instead of the saved login:

```shell
export CONFLUENCE_BASE_URL=https://example.atlassian.net
export CONFLUENCE_EMAIL=bot@example.com
export CONFLUENCE_API_KEY=...
```

Or log in without prompts, reading the token from standard input:

```shell
echo "$TOKEN" | rfluence auth login --email bot@example.com --with-token
```
