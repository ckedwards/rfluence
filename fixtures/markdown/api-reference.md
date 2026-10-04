# Events API Reference

## Install & Setup (v2.0)

Base URL: `https://api.example.com/v1`

## FAQ: What's new?

Version 2 adds batch endpoints. See [Install & Setup](#install--setup-v20).

## Endpoints

| Method | Path | Description |
| --- | --- | --- |
| `GET` | `/events/{id}` | Fetch a single event |
| `POST` | `/events` | Create events (max 500 per batch) |
| `GET` | `/events?filter=a\|b` | Filter with `a\|b` (OR) |
| `DELETE` | `/events/{id}` | Soft-delete; see notes below |

### Filter syntax

| Operator | Example | Meaning |
|:---|:---:|---:|
| `\|` | `type:a\|type:b` | OR |
| `&` | `type:a&status:ok` | AND |
| `!` | `!status:failed` | NOT |

## Errors

| Code | Name | Retry? |
|------|------|--------|
| 400 | `bad_request` | No |
| 429 | `rate_limited` | Yes, after `Retry-After` |
| 503 | `unavailable` | Yes, with **exponential** backoff |

## Example

```json
{
  "id": "evt_123",
  "type": "order.created",
  "payload": { "order_id": 42, "total": "19.99" }
}
```

```http
POST /v1/events HTTP/1.1
Content-Type: application/json
```

## Duplicate

First section with this heading.

## Duplicate

Second section with the same heading; GitHub anchors it as `#duplicate-1`, Confluence as `#Duplicate.1`.

## Extra   spaces

Heading with repeated spaces.

## Ünïcode Héading

Links: [first duplicate](#duplicate), [second duplicate](#duplicate-1), [unicode](#ünïcode-héading), [FAQ](#faq-whats-new).
