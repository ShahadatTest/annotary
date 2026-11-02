# Fernway Maps API reference

Version 2. Everything here is served from `https://api.example.com`, except the tile endpoints, which customers may also reach through `https://tiles.example.com`. Questions go to `support@example.com`.

## Conventions

Requests and responses are JSON unless the endpoint says otherwise. Timestamps are RFC 3339 in UTC. Identifiers are opaque strings, and you should not parse them. Unknown fields may be added to any response at any time, and a client must ignore them.

### Pagination

List endpoints take `limit` and `cursor`, and return `next_cursor` while there is more. A cursor stays valid for 24 hours.

```bash
curl "https://api.example.com/v2/styles?limit=2&cursor=eyJwIjoxfQ" -H "Authorization: Bearer $FERNWAY_KEY"
```

### Rate limits

Every response carries `X-RateLimit-Remaining`. Over the limit, a request gets a 429 with `Retry-After`. Tile requests have a much higher limit than everything else, because a single screen of map is about twenty of them.

## Authentication

Send a key as a bearer token. Tile URLs may also carry it as `?key=`, since an `<img>` tag cannot set a header; a key used that way should be a public `pk_` key restricted by origin.

```http
GET /v2/styles HTTP/1.1
Host: api.example.com
Authorization: Bearer sk_test_0000000000000000
```

## Errors

Errors share one shape, and `request_id` is what support will ask you for:

```json
{
  "error": "invalid_field",
  "message": "width must be between 64 and 2048",
  "field": "width",
  "request_id": "req_01hzy8m4"
}
```

| Status | Error | Meaning | What to do |
|---|---|---|---|
| `400` | `bad_request` | The request could not be parsed. | Check the parameters against this page. |
| `401` | `invalid_key` | The API key is missing, unknown or revoked. | Send a live key in `Authorization`. |
| `403` | `key_restricted` | The key is valid but not allowed from this origin. | Add the origin to the key. |
| `404` | `not_found` | Nothing exists at this path. | Check the identifier. |
| `409` | `conflict` | The resource changed since you read it. | Read it again and retry. |
| `422` | `invalid_field` | A field failed validation; `field` names it. | Correct the field. |
| `429` | `rate_limited` | The key is over its limit. | Wait for the time in `Retry-After`. |
| `500` | `internal` | Something failed on our side. | Retry with backoff; quote `request_id`. |
| `503` | `overloaded` | The render queue is full. | Wait for the time in `Retry-After`. |
| `404` | `tile_y}_missing` | `GET /v2/tiles/{style}/{z}/{x}/{y}.png`: nothing found. | Check the identifier. |
| `404` | `tile_y}_missing` | `GET /v2/tiles/{style}/{z}/{x}/{y}.mvt`: nothing found. | Check the identifier. |
| `404` | `tile_tilejson_missing` | `GET /v2/tiles/{style}/tilejson.json`: nothing found. | Check the identifier. |
| `404` | `tile_style_missing` | `GET /v2/static/{style}`: nothing found. | Check the identifier. |
| `404` | `tile_purge_missing` | `POST /v2/tiles/purge`: nothing found. | Check the identifier. |
| `409` | `tile_purge_conflict` | `POST /v2/tiles/purge`: changed meanwhile. | Read it again and retry. |
| `404` | `style_styles_missing` | `GET /v2/styles`: nothing found. | Check the identifier. |
| `404` | `style_style_missing` | `GET /v2/styles/{style}`: nothing found. | Check the identifier. |
| `404` | `style_style_missing` | `PUT /v2/styles/{style}`: nothing found. | Check the identifier. |
| `409` | `style_style_conflict` | `PUT /v2/styles/{style}`: changed meanwhile. | Read it again and retry. |
| `404` | `style_publish_missing` | `POST /v2/styles/{style}/publish`: nothing found. | Check the identifier. |
| `409` | `style_publish_conflict` | `POST /v2/styles/{style}/publish`: changed meanwhile. | Read it again and retry. |
| `404` | `style_rollback_missing` | `POST /v2/styles/{style}/rollback`: nothing found. | Check the identifier. |
| `409` | `style_rollback_conflict` | `POST /v2/styles/{style}/rollback`: changed meanwhile. | Read it again and retry. |
| `404` | `style_name}_missing` | `GET /v2/styles/{style}/sprites/{name}.png`: nothing found. | Check the identifier. |
| `404` | `style_revisions_missing` | `GET /v2/styles/{style}/revisions`: nothing found. | Check the identifier. |
| `404` | `dataset_datasets_missing` | `GET /v2/datasets`: nothing found. | Check the identifier. |
| `404` | `dataset_id_missing` | `GET /v2/datasets/{id}`: nothing found. | Check the identifier. |
| `404` | `dataset_overlays_missing` | `POST /v2/overlays`: nothing found. | Check the identifier. |
| `409` | `dataset_overlays_conflict` | `POST /v2/overlays`: changed meanwhile. | Read it again and retry. |
| `404` | `dataset_id_missing` | `GET /v2/overlays/{id}`: nothing found. | Check the identifier. |
| `404` | `dataset_id_missing` | `DELETE /v2/overlays/{id}`: nothing found. | Check the identifier. |
| `409` | `dataset_id_conflict` | `DELETE /v2/overlays/{id}`: changed meanwhile. | Read it again and retry. |
| `404` | `dataset_changes_missing` | `GET /v2/datasets/{id}/changes`: nothing found. | Check the identifier. |
| `404` | `geocoding_geocode_missing` | `GET /v2/geocode`: nothing found. | Check the identifier. |
| `404` | `geocoding_reverse_missing` | `GET /v2/reverse`: nothing found. | Check the identifier. |
| `404` | `geocoding_elevation_missing` | `GET /v2/elevation`: nothing found. | Check the identifier. |
| `404` | `geocoding_profile_missing` | `POST /v2/route/profile`: nothing found. | Check the identifier. |
| `409` | `geocoding_profile_conflict` | `POST /v2/route/profile`: changed meanwhile. | Read it again and retry. |
| `404` | `key_keys_missing` | `GET /v2/keys`: nothing found. | Check the identifier. |
| `404` | `key_keys_missing` | `POST /v2/keys`: nothing found. | Check the identifier. |
| `409` | `key_keys_conflict` | `POST /v2/keys`: changed meanwhile. | Read it again and retry. |
| `404` | `key_id_missing` | `PATCH /v2/keys/{id}`: nothing found. | Check the identifier. |
| `409` | `key_id_conflict` | `PATCH /v2/keys/{id}`: changed meanwhile. | Read it again and retry. |
| `404` | `key_id_missing` | `DELETE /v2/keys/{id}`: nothing found. | Check the identifier. |
| `409` | `key_id_conflict` | `DELETE /v2/keys/{id}`: changed meanwhile. | Read it again and retry. |
| `404` | `key_rotate_missing` | `POST /v2/keys/{id}/rotate`: nothing found. | Check the identifier. |
| `409` | `key_rotate_conflict` | `POST /v2/keys/{id}/rotate`: changed meanwhile. | Read it again and retry. |
| `404` | `usage_usage_missing` | `GET /v2/usage`: nothing found. | Check the identifier. |
| `404` | `usage_top_tiles_missing` | `GET /v2/usage/top-tiles`: nothing found. | Check the identifier. |
| `404` | `usage_limits_missing` | `GET /v2/usage/limits`: nothing found. | Check the identifier. |
| `404` | `webhook_webhooks_missing` | `GET /v2/webhooks`: nothing found. | Check the identifier. |
| `404` | `webhook_webhooks_missing` | `POST /v2/webhooks`: nothing found. | Check the identifier. |
| `409` | `webhook_webhooks_conflict` | `POST /v2/webhooks`: changed meanwhile. | Read it again and retry. |
| `404` | `webhook_id_missing` | `DELETE /v2/webhooks/{id}`: nothing found. | Check the identifier. |
| `409` | `webhook_id_conflict` | `DELETE /v2/webhooks/{id}`: changed meanwhile. | Read it again and retry. |
| `404` | `webhook_test_missing` | `POST /v2/webhooks/{id}/test`: nothing found. | Check the identifier. |
| `409` | `webhook_test_conflict` | `POST /v2/webhooks/{id}/test`: changed meanwhile. | Read it again and retry. |
| `404` | `webhook_deliveries_missing` | `GET /v2/webhooks/{id}/deliveries`: nothing found. | Check the identifier. |
| `404` | `print job_jobs_missing` | `POST /v2/print/jobs`: nothing found. | Check the identifier. |
| `409` | `print job_jobs_conflict` | `POST /v2/print/jobs`: changed meanwhile. | Read it again and retry. |
| `404` | `print job_jobs_missing` | `GET /v2/print/jobs`: nothing found. | Check the identifier. |
| `404` | `print job_id_missing` | `GET /v2/print/jobs/{id}`: nothing found. | Check the identifier. |
| `404` | `print job_id_missing` | `DELETE /v2/print/jobs/{id}`: nothing found. | Check the identifier. |
| `409` | `print job_id_conflict` | `DELETE /v2/print/jobs/{id}`: changed meanwhile. | Read it again and retry. |
| `404` | `print job_presets_missing` | `GET /v2/print/presets`: nothing found. | Check the identifier. |
| `404` | `survey_surveys_missing` | `GET /v2/surveys`: nothing found. | Check the identifier. |
| `404` | `survey_surveys_missing` | `POST /v2/surveys`: nothing found. | Check the identifier. |
| `409` | `survey_surveys_conflict` | `POST /v2/surveys`: changed meanwhile. | Read it again and retry. |
| `404` | `survey_id_missing` | `GET /v2/surveys/{id}`: nothing found. | Check the identifier. |
| `404` | `survey_tracks_missing` | `POST /v2/surveys/{id}/tracks`: nothing found. | Check the identifier. |
| `409` | `survey_tracks_conflict` | `POST /v2/surveys/{id}/tracks`: changed meanwhile. | Read it again and retry. |
| `404` | `survey_photos_missing` | `POST /v2/surveys/{id}/photos`: nothing found. | Check the identifier. |
| `409` | `survey_photos_conflict` | `POST /v2/surveys/{id}/photos`: changed meanwhile. | Read it again and retry. |
| `404` | `survey_review_missing` | `POST /v2/surveys/{id}/review`: nothing found. | Check the identifier. |
| `409` | `survey_review_conflict` | `POST /v2/surveys/{id}/review`: changed meanwhile. | Read it again and retry. |
| `404` | `survey_diff_missing` | `GET /v2/surveys/{id}/diff`: nothing found. | Check the identifier. |
| `404` | `customer_customer_missing` | `GET /v2/customer`: nothing found. | Check the identifier. |
| `404` | `customer_customer_missing` | `PATCH /v2/customer`: nothing found. | Check the identifier. |
| `409` | `customer_customer_conflict` | `PATCH /v2/customer`: changed meanwhile. | Read it again and retry. |
| `404` | `customer_invoices_missing` | `GET /v2/customer/invoices`: nothing found. | Check the identifier. |
| `404` | `customer_attribution_missing` | `GET /v2/customer/attribution`: nothing found. | Check the identifier. |
| `404` | `customer_members_missing` | `GET /v2/customer/members`: nothing found. | Check the identifier. |
| `404` | `customer_members_missing` | `POST /v2/customer/members`: nothing found. | Check the identifier. |
| `409` | `customer_members_conflict` | `POST /v2/customer/members`: changed meanwhile. | Read it again and retry. |

## Regions

| Region | City | Mostly serves | Edge nodes |
|---|---|---|---|
| eu-north | Stockholm | Fernway's first region | 14 |
| eu-west | Dublin | Newspapers | 9 |
| eu-south | Milan | Greek and Italian ferries | 5 |
| me-south | Bahrain | Gulf ferries | 3 |
| il-central | Tel Aviv | Hiking clubs | 2 |
| ap-northeast | Tokyo | Japanese clubs | 4 |

## Tiles

Tiles are the product. The tile URL is stable by contract: customers hard-code it into apps that may not be updated for years, so nothing in this section changes without a new version prefix.

### Endpoints

#### GET `/v2/tiles/{style}/{z}/{x}/{y}.png`

Fetch a raster tile.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `style` | string | A style identifier, such as `outdoor`, `nautical` or `atlas-print`. |
| `z` | integer | The zoom level, from 0 to 18. |
| `x` | integer | The column, counted from the west, from 0 to 2^z - 1. |
| `y` | integer | The row, counted from the north, from 0 to 2^z - 1. |

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `lang` | string | The label language as a BCP 47 tag. Defaults to the customer's setting. |
| `scale` | integer | 1 or 2, for high-density screens. Defaults to 1. |

##### Example request

```bash
curl "https://api.example.com/v2/tiles/outdoor/12/2189/1273.png" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `(body)` | image/png | The tile, 256 by 256 logical pixels. |

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/tiles/{style}/{z}/{x}/{y}.png`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/tiles/{style}/{z}/{x}/{y}.mvt`

Fetch a vector tile.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `style` | string | A style identifier, such as `outdoor`, `nautical` or `atlas-print`. |
| `z` | integer | The zoom level, from 0 to 18. |
| `x` | integer | The column, counted from the west, from 0 to 2^z - 1. |
| `y` | integer | The row, counted from the north, from 0 to 2^z - 1. |

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `lang` | string | The label language as a BCP 47 tag. Defaults to the customer's setting. |

##### Example request

```bash
curl "https://api.example.com/v2/tiles/outdoor/12/2189/1273.mvt" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `(body)` | application/vnd.mapbox-vector-tile | The tile, gzip-encoded. |

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/tiles/{style}/{z}/{x}/{y}.mvt`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/tiles/{style}/tilejson.json`

Describe a style's tiles.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `style` | string | A style identifier, such as `outdoor`, `nautical` or `atlas-print`. |

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `lang` | string | The label language as a BCP 47 tag. Defaults to the customer's setting. |

##### Example request

```bash
curl "https://api.example.com/v2/tiles/outdoor/tilejson.json" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `tiles` | array | URL templates. |
| `minzoom` | integer | The lowest zoom served. |
| `maxzoom` | integer | The highest zoom served. |
| `attribution` | string | HTML the customer must display. |

###### Example

```json
{
  "tiles": [],
  "minzoom": 48,
  "maxzoom": 48,
  "attribution": "outdoor"
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/tiles/{style}/tilejson.json`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/static/{style}`

Render a static map image.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `style` | string | A style identifier, such as `outdoor`, `nautical` or `atlas-print`. |

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `bbox` | string | West, south, east and north in degrees, comma-separated. |
| `width` | integer | Pixels, from 64 to 2048. |
| `height` | integer | Pixels, from 64 to 2048. |
| `markers` | string | Up to 50 points as `lon,lat` pairs, separated by `;`. |

##### Example request

```bash
curl "https://api.example.com/v2/static/outdoor" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `(body)` | image/png | The picture, with attribution drawn in. |

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/static/{style}`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/tiles/purge`

Purge tiles from every cache tier.

##### Parameters

| Name | Type | Description |
|---|---|---|
| `reason` | string | Why, for the audit log. Required. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/tiles/purge" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `tiles` | integer | How many tiles were removed. |
| `audit_id` | string | The audit line's identifier. |

###### Example

```json
{
  "tiles": 48,
  "audit_id": "outdoor"
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/tiles/purge`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

## Styles

A style is a set of drawing rules with a revision counter. Publishing bumps the counter; rolling back moves it down. Old revisions stay available for 30 days.

### Endpoints

#### GET `/v2/styles`

List the styles a key can use.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `limit` | integer | How many items to return, from 1 to 200. Defaults to 50. |
| `cursor` | string | An opaque position from an earlier response's `next_cursor`. |

##### Example request

```bash
curl "https://api.example.com/v2/styles" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `styles` | array | Style summaries. |
| `next_cursor` | string | Present when there is more. |

###### Example

```json
{
  "styles": [],
  "next_cursor": "outdoor"
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/styles`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/styles/{style}`

Read a style.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `style` | string | A style identifier, such as `outdoor`, `nautical` or `atlas-print`. |

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `rev` | integer | A specific revision. Defaults to the active one. |

##### Example request

```bash
curl "https://api.example.com/v2/styles/outdoor" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `id` | string | The identifier. |
| `rev` | integer | The revision. |
| `published_at` | timestamp | When it went live. |
| `published_by` | string | Who published it. |
| `layers` | array | The drawing rules. |

###### Example

```json
{
  "id": "outdoor",
  "rev": 48,
  "published_at": "2026-03-02T09:30:00Z",
  "published_by": "outdoor",
  "layers": []
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/styles/{style}`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### PUT `/v2/styles/{style}`

Create or replace a draft.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `style` | string | A style identifier, such as `outdoor`, `nautical` or `atlas-print`. |

##### Example request

```bash
curl -X PUT "https://api.example.com/v2/styles/outdoor" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `id` | string | The identifier. |
| `draft_rev` | integer | The new draft's number. |
| `warnings` | array | Lint findings; a draft can have them. |

###### Example

```json
{
  "id": "outdoor",
  "draft_rev": 48,
  "warnings": []
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/styles/{style}`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/styles/{style}/publish`

Publish the current draft.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `style` | string | A style identifier, such as `outdoor`, `nautical` or `atlas-print`. |

##### Parameters

| Name | Type | Description |
|---|---|---|
| `region` | string | One region, or all of them when left out. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/styles/outdoor/publish" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `rev` | integer | The revision now active. |
| `previous_rev` | integer | What it replaced. |

###### Example

```json
{
  "rev": 48,
  "previous_rev": 48
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/styles/{style}/publish`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/styles/{style}/rollback`

Make an earlier revision active.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `style` | string | A style identifier, such as `outdoor`, `nautical` or `atlas-print`. |

##### Parameters

| Name | Type | Description |
|---|---|---|
| `to_rev` | integer | The revision. Defaults to the one before. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/styles/outdoor/rollback" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `rev` | integer | The revision now active. |

###### Example

```json
{
  "rev": 48
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/styles/{style}/rollback`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/styles/{style}/sprites/{name}.png`

Fetch a sprite sheet.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `style` | string | A style identifier, such as `outdoor`, `nautical` or `atlas-print`. |
| `name` | string | `sprite` or `sprite@2x`. |

##### Example request

```bash
curl "https://api.example.com/v2/styles/outdoor/sprites/sprite.png" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `(body)` | image/png | The sheet. |

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/styles/{style}/sprites/{name}.png`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/styles/{style}/revisions`

List revisions, newest first.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `style` | string | A style identifier, such as `outdoor`, `nautical` or `atlas-print`. |

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `limit` | integer | How many items to return, from 1 to 200. Defaults to 50. |
| `cursor` | string | An opaque position from an earlier response's `next_cursor`. |

##### Example request

```bash
curl "https://api.example.com/v2/styles/outdoor/revisions" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `revisions` | array | Each with `rev`, `published_at` and `published_by`. |

###### Example

```json
{
  "revisions": []
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/styles/{style}/revisions`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

## Datasets

A dataset is the output of one nightly data build. Customers cannot write to the base datasets, but can upload overlays, such as a club's own trail network.

### Endpoints

#### GET `/v2/datasets`

List datasets.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `limit` | integer | How many items to return, from 1 to 200. Defaults to 50. |
| `cursor` | string | An opaque position from an earlier response's `next_cursor`. |
| `since` | timestamp | Only items changed after this time, in RFC 3339 form. |

##### Example request

```bash
curl "https://api.example.com/v2/datasets" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `datasets` | array | Dataset summaries. |
| `next_cursor` | string | Present when there is more. |

###### Example

```json
{
  "datasets": [],
  "next_cursor": "outdoor"
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/datasets`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/datasets/{id}`

Read a dataset.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The dataset's identifier. |

##### Example request

```bash
curl "https://api.example.com/v2/datasets/ds_01hzy8" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `id` | string | The identifier. |
| `build_date` | date | When it was built. |
| `sources` | array | Attribution for each source. |
| `feature_count` | integer | How many features. |

###### Example

```json
{
  "id": "outdoor",
  "build_date": "2026-03-02",
  "sources": [],
  "feature_count": 48
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/datasets/{id}`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/overlays`

Upload an overlay as GeoJSON.

##### Parameters

| Name | Type | Description |
|---|---|---|
| `name` | string | A display name. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/overlays" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `id` | string | The overlay's identifier. |
| `features` | integer | How many were accepted. |
| `rejected` | array | Features refused, with reasons. |

###### Example

```json
{
  "id": "outdoor",
  "features": 48,
  "rejected": []
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/overlays`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/overlays/{id}`

Read an overlay.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The overlay's identifier. |

##### Example request

```bash
curl "https://api.example.com/v2/overlays/ds_01hzy8" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `id` | string | The identifier. |
| `name` | string | The display name. |
| `features` | integer | How many. |

###### Example

```json
{
  "id": "outdoor",
  "name": "outdoor",
  "features": 48
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/overlays/{id}`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### DELETE `/v2/overlays/{id}`

Delete an overlay.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The overlay's identifier. |

##### Example request

```bash
curl -X DELETE "https://api.example.com/v2/overlays/ds_01hzy8" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `deleted` | boolean | Always true. |

###### Example

```json
{
  "deleted": true
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/overlays/{id}`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/datasets/{id}/changes`

List what changed since the previous build.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The dataset's identifier. |

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `bbox` | string | West, south, east and north in degrees, comma-separated. |
| `limit` | integer | How many items to return, from 1 to 200. Defaults to 50. |
| `cursor` | string | An opaque position from an earlier response's `next_cursor`. |

##### Example request

```bash
curl "https://api.example.com/v2/datasets/ds_01hzy8/changes" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `changes` | array | Each with `kind`, `feature` and `geometry`. |
| `next_cursor` | string | Present when there is more. |

###### Example

```json
{
  "changes": [],
  "next_cursor": "outdoor"
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/datasets/{id}/changes`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

## Geocoding

Search for places by name and turn coordinates back into names. Results are ranked by distance from `near` when it is given, and by prominence otherwise.

### Endpoints

#### GET `/v2/geocode`

Search for a place by name.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `q` | string | The search text, in any script. Required. |
| `near` | string | `lon,lat` to rank around. |
| `lang` | string | The label language as a BCP 47 tag. Defaults to the customer's setting. |
| `limit` | integer | How many items to return, from 1 to 200. Defaults to 50. |

##### Example request

```bash
curl "https://api.example.com/v2/geocode" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `results` | array | Places, best first. |
| `results[].name` | string | The display name. |
| `results[].lon` | number | Longitude. |
| `results[].lat` | number | Latitude. |
| `results[].kind` | string | `harbour`, `summit`, `trail`, `town` and so on. |

###### Example

```json
{
  "results": []
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/geocode`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/reverse`

Name the place at a coordinate.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `lon` | number | Longitude. Required. |
| `lat` | number | Latitude. Required. |
| `lang` | string | The label language as a BCP 47 tag. Defaults to the customer's setting. |

##### Example request

```bash
curl "https://api.example.com/v2/reverse" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `name` | string | The display name. |
| `distance_m` | number | How far away it is. |

###### Example

```json
{
  "name": "outdoor",
  "distance_m": 57.7
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/reverse`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/elevation`

Read the height at a coordinate.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `points` | string | Up to 200 `lon,lat` pairs separated by `;`. Required. |

##### Example request

```bash
curl "https://api.example.com/v2/elevation" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `heights` | array | Metres above mean sea level, in order. |

###### Example

```json
{
  "heights": []
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/elevation`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/route/profile`

Return a trail's height profile.

##### Parameters

| Name | Type | Description |
|---|---|---|
| `sample_m` | integer | Metres between samples. Defaults to 25. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/route/profile" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `distance_m` | number | The total length. |
| `ascent_m` | number | The total climb. |
| `samples` | array | `[distance, height]` pairs. |

###### Example

```json
{
  "distance_m": 57.7,
  "ascent_m": 57.7,
  "samples": []
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/route/profile`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

## Keys

API keys belong to a customer. A `pk_` key is public and restricted by origin; an `sk_` key is secret and must stay on a server. Only a secret key can manage keys.

### Endpoints

#### GET `/v2/keys`

List a customer's keys.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `limit` | integer | How many items to return, from 1 to 200. Defaults to 50. |
| `cursor` | string | An opaque position from an earlier response's `next_cursor`. |

##### Example request

```bash
curl "https://api.example.com/v2/keys" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `keys` | array | Keys, with the secret part hidden. |
| `next_cursor` | string | Present when there is more. |

###### Example

```json
{
  "keys": [],
  "next_cursor": "outdoor"
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/keys`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/keys`

Create a key.

##### Parameters

| Name | Type | Description |
|---|---|---|
| `kind` | string | `public` or `secret`. |
| `name` | string | A label. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/keys" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `id` | string | The identifier. |
| `key` | string | The key itself, shown this once. |
| `origins` | array | Allowed origins; empty means none. |

###### Example

```json
{
  "id": "outdoor",
  "key": "outdoor",
  "origins": []
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/keys`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### PATCH `/v2/keys/{id}`

Change a key's name or origins.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The key's identifier. |

##### Example request

```bash
curl -X PATCH "https://api.example.com/v2/keys/ds_01hzy8" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `id` | string | The identifier. |
| `name` | string | The label. |
| `origins` | array | Allowed origins. |

###### Example

```json
{
  "id": "outdoor",
  "name": "outdoor",
  "origins": []
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/keys/{id}`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### DELETE `/v2/keys/{id}`

Revoke a key at once.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The key's identifier. |

##### Example request

```bash
curl -X DELETE "https://api.example.com/v2/keys/ds_01hzy8" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `revoked_at` | timestamp | When it stopped working. |

###### Example

```json
{
  "revoked_at": "2026-03-02T09:30:00Z"
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/keys/{id}`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/keys/{id}/rotate`

Replace a key, keeping the old one alive for a while.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The key's identifier. |

##### Parameters

| Name | Type | Description |
|---|---|---|
| `overlap_hours` | integer | How long both work, up to 168. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/keys/ds_01hzy8/rotate" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `key` | string | The new key, shown this once. |
| `old_key_expires_at` | timestamp | When the old one stops. |

###### Example

```json
{
  "key": "outdoor",
  "old_key_expires_at": "2026-03-02T09:30:00Z"
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/keys/{id}/rotate`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

## Usage

Usage is counted per key and per day, in UTC. A day's figures settle about two hours after midnight, so today's numbers are estimates.

### Endpoints

#### GET `/v2/usage`

Read usage per day.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `from` | date | The first day. |
| `to` | date | The last day, inclusive. |
| `key` | string | Only this key. |

##### Example request

```bash
curl "https://api.example.com/v2/usage" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `days` | array | Each with `date`, `tiles`, `static` and `geocode`. |
| `settled_until` | date | The last settled day. |

###### Example

```json
{
  "days": [],
  "settled_until": "2026-03-02"
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/usage`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/usage/top-tiles`

List the most requested tiles.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `days` | integer | How far back, up to 30. |
| `limit` | integer | How many items to return, from 1 to 200. Defaults to 50. |

##### Example request

```bash
curl "https://api.example.com/v2/usage/top-tiles" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `tiles` | array | Each with `z`, `x`, `y` and a count above the privacy threshold. |

###### Example

```json
{
  "tiles": []
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/usage/top-tiles`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/usage/limits`

Read the key's limits.

##### Example request

```bash
curl "https://api.example.com/v2/usage/limits" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `requests_per_minute` | integer | The burst limit. |
| `tiles_per_month` | integer | The contract's allowance. |
| `used_this_month` | integer | So far. |

###### Example

```json
{
  "requests_per_minute": 48,
  "tiles_per_month": 48,
  "used_this_month": 48
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/usage/limits`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

## Webhooks

A webhook tells a customer's server that something changed. Deliveries are signed, retried for a day with backoff, and may arrive more than once, so a receiver should use `event_id` to ignore repeats.

### Endpoints

#### GET `/v2/webhooks`

List webhooks.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `limit` | integer | How many items to return, from 1 to 200. Defaults to 50. |
| `cursor` | string | An opaque position from an earlier response's `next_cursor`. |

##### Example request

```bash
curl "https://api.example.com/v2/webhooks" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `webhooks` | array | Each with `id`, `url` and `events`. |
| `next_cursor` | string | Present when there is more. |

###### Example

```json
{
  "webhooks": [],
  "next_cursor": "outdoor"
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/webhooks`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/webhooks`

Create a webhook.

##### Parameters

| Name | Type | Description |
|---|---|---|
| `url` | string | An HTTPS address. |
| `events` | array | `style.published`, `dataset.built` or `overlay.ready`. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/webhooks" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `id` | string | The identifier. |
| `secret` | string | The signing secret, shown this once. |

###### Example

```json
{
  "id": "outdoor",
  "secret": "outdoor"
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/webhooks`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### DELETE `/v2/webhooks/{id}`

Delete a webhook.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The webhook's identifier. |

##### Example request

```bash
curl -X DELETE "https://api.example.com/v2/webhooks/ds_01hzy8" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `deleted` | boolean | Always true. |

###### Example

```json
{
  "deleted": true
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/webhooks/{id}`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/webhooks/{id}/test`

Send a test delivery now.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The webhook's identifier. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/webhooks/ds_01hzy8/test" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `delivered` | boolean | Whether the server answered 2xx. |
| `status` | integer | What it answered. |

###### Example

```json
{
  "delivered": true,
  "status": 48
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/webhooks/{id}/test`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/webhooks/{id}/deliveries`

List recent deliveries.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The webhook's identifier. |

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `limit` | integer | How many items to return, from 1 to 200. Defaults to 50. |
| `cursor` | string | An opaque position from an earlier response's `next_cursor`. |

##### Example request

```bash
curl "https://api.example.com/v2/webhooks/ds_01hzy8/deliveries" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `deliveries` | array | Each with `event_id`, `status` and `attempts`. |

###### Example

```json
{
  "deliveries": []
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/webhooks/{id}/deliveries`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

## Print jobs

A print job turns a map area into a press-ready PDF at a fixed scale. Jobs run in the background and can take minutes, so the API hands back an identifier and a webhook or a poll says when the file is ready.

### Endpoints

#### POST `/v2/print/jobs`

Start a print job.

##### Parameters

| Name | Type | Description |
|---|---|---|
| `bbox` | string | West, south, east and north in degrees, comma-separated. |
| `style` | string | The style, usually `atlas-print`. |
| `scale` | integer | The denominator, such as 25000. |
| `paper` | string | `A0` to `A4`. |
| `bleed_mm` | integer | Bleed in millimetres. Defaults to 3. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/print/jobs" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `id` | string | The job's identifier. |
| `state` | string | `queued`, `running`, `done` or `failed`. |
| `estimated_seconds` | integer | A rough guess. |

###### Example

```json
{
  "id": "outdoor",
  "state": "outdoor",
  "estimated_seconds": 48
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/print/jobs`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/print/jobs`

List print jobs.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `limit` | integer | How many items to return, from 1 to 200. Defaults to 50. |
| `cursor` | string | An opaque position from an earlier response's `next_cursor`. |
| `since` | timestamp | Only items changed after this time, in RFC 3339 form. |

##### Example request

```bash
curl "https://api.example.com/v2/print/jobs" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `jobs` | array | Job summaries. |
| `next_cursor` | string | Present when there is more. |

###### Example

```json
{
  "jobs": [],
  "next_cursor": "outdoor"
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/print/jobs`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/print/jobs/{id}`

Read a print job.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The print job's identifier. |

##### Example request

```bash
curl "https://api.example.com/v2/print/jobs/ds_01hzy8" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `id` | string | The identifier. |
| `state` | string | Where it has got to. |
| `progress` | number | From 0 to 1. |
| `file_url` | string | A signed link, once done. |
| `expires_at` | timestamp | When the link stops working. |

###### Example

```json
{
  "id": "outdoor",
  "state": "outdoor",
  "progress": 57.7,
  "file_url": "outdoor",
  "expires_at": "2026-03-02T09:30:00Z"
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/print/jobs/{id}`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### DELETE `/v2/print/jobs/{id}`

Cancel a job, or delete its file.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The print job's identifier. |

##### Example request

```bash
curl -X DELETE "https://api.example.com/v2/print/jobs/ds_01hzy8" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `state` | string | `cancelled` or `deleted`. |

###### Example

```json
{
  "state": "outdoor"
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/print/jobs/{id}`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/print/presets`

List paper and scale presets.

##### Example request

```bash
curl "https://api.example.com/v2/print/presets" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `presets` | array | Each with `name`, `paper`, `scale` and `margins_mm`. |

###### Example

```json
{
  "presets": []
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/print/presets`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

## Surveys

Survey sessions hold what field teams record: tracks, photos and notes. They feed the next data build once somebody has reviewed them. Customers with a field-data contract can read their own sessions.

### Endpoints

#### GET `/v2/surveys`

List survey sessions.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `bbox` | string | West, south, east and north in degrees, comma-separated. |
| `limit` | integer | How many items to return, from 1 to 200. Defaults to 50. |
| `cursor` | string | An opaque position from an earlier response's `next_cursor`. |
| `since` | timestamp | Only items changed after this time, in RFC 3339 form. |

##### Example request

```bash
curl "https://api.example.com/v2/surveys" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `sessions` | array | Session summaries. |
| `next_cursor` | string | Present when there is more. |

###### Example

```json
{
  "sessions": [],
  "next_cursor": "outdoor"
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/surveys`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/surveys`

Start a survey session.

##### Parameters

| Name | Type | Description |
|---|---|---|
| `name` | string | A label, such as `Skåne coast, week 14`. |
| `device` | string | The handheld's serial number. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/surveys" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `id` | string | The session's identifier. |
| `upload_url` | string | Where to send the tracks. |

###### Example

```json
{
  "id": "outdoor",
  "upload_url": "outdoor"
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/surveys`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/surveys/{id}`

Read a survey session.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The survey session's identifier. |

##### Example request

```bash
curl "https://api.example.com/v2/surveys/ds_01hzy8" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `id` | string | The identifier. |
| `name` | string | The label. |
| `track_km` | number | The length recorded. |
| `photos` | integer | How many photos. |
| `reviewed_by` | string | Who reviewed it, if anyone. |

###### Example

```json
{
  "id": "outdoor",
  "name": "outdoor",
  "track_km": 57.7,
  "photos": 48,
  "reviewed_by": "outdoor"
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/surveys/{id}`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/surveys/{id}/tracks`

Upload a track as GPX.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The survey session's identifier. |

##### Parameters

| Name | Type | Description |
|---|---|---|
| `accuracy_m` | number | The handheld's reported accuracy. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/surveys/ds_01hzy8/tracks" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `points` | integer | How many were accepted. |
| `rejected` | integer | How many were dropped as noise. |

###### Example

```json
{
  "points": 48,
  "rejected": 48
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/surveys/{id}/tracks`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/surveys/{id}/photos`

Attach a geotagged photo.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The survey session's identifier. |

##### Parameters

| Name | Type | Description |
|---|---|---|
| `lon` | number | Overrides the photo's own tag. |
| `lat` | number | Overrides the photo's own tag. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/surveys/ds_01hzy8/photos" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `photo_id` | string | The photo's identifier. |
| `thumbnail_url` | string | A small copy. |

###### Example

```json
{
  "photo_id": "outdoor",
  "thumbnail_url": "outdoor"
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/surveys/{id}/photos`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/surveys/{id}/review`

Approve a session for the next build.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The survey session's identifier. |

##### Parameters

| Name | Type | Description |
|---|---|---|
| `note` | string | Anything the build should know. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/surveys/ds_01hzy8/review" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `state` | string | `approved`. |
| `next_build` | date | When it will be included. |

###### Example

```json
{
  "state": "outdoor",
  "next_build": "2026-03-02"
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/surveys/{id}/review`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/surveys/{id}/diff`

Compare a session with the current data.

##### Path parameters

| Name | Type | Description |
|---|---|---|
| `id` | string | The survey session's identifier. |

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `buffer_m` | integer | How far a track may stray before it counts as new. Defaults to 10. |

##### Example request

```bash
curl "https://api.example.com/v2/surveys/ds_01hzy8/diff" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `added` | array | Features the survey has that the data does not. |
| `missing` | array | Features the data has that the survey did not find. |
| `moved` | array | Features displaced by more than the buffer. |

###### Example

```json
{
  "added": [],
  "missing": [],
  "moved": []
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/surveys/{id}/diff`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

## Customers

Account details for the customer the key belongs to. Only a secret key can read these, and nothing in this section is ever cached at the edge.

### Endpoints

#### GET `/v2/customer`

Read the customer's account.

##### Example request

```bash
curl "https://api.example.com/v2/customer" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `id` | string | The identifier. |
| `name` | string | The organisation. |
| `contract` | string | `club`, `standard` or `enterprise`. |
| `default_lang` | string | The label language. |
| `regions` | array | Where its tiles are served from. |

###### Example

```json
{
  "id": "outdoor",
  "name": "outdoor",
  "contract": "outdoor",
  "default_lang": "outdoor",
  "regions": []
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/customer`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### PATCH `/v2/customer`

Change the account's settings.

##### Parameters

| Name | Type | Description |
|---|---|---|
| `default_lang` | string | The label language. |

##### Example request

```bash
curl -X PATCH "https://api.example.com/v2/customer" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `default_lang` | string | As stored. |
| `updated_at` | timestamp | When it changed. |

###### Example

```json
{
  "default_lang": "outdoor",
  "updated_at": "2026-03-02T09:30:00Z"
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/customer`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/customer/invoices`

List invoices.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `limit` | integer | How many items to return, from 1 to 200. Defaults to 50. |
| `cursor` | string | An opaque position from an earlier response's `next_cursor`. |

##### Example request

```bash
curl "https://api.example.com/v2/customer/invoices" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `invoices` | array | Each with `number`, `amount`, `currency`, `paid_at` and `pdf_url`. |

###### Example

```json
{
  "invoices": []
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/customer/invoices`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/customer/attribution`

Read what the customer must display.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `style` | string | One style, or all of them. |

##### Example request

```bash
curl "https://api.example.com/v2/customer/attribution" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `html` | string | Ready to paste. |
| `text` | string | For print. |
| `sources` | array | Each source and its licence. |

###### Example

```json
{
  "html": "outdoor",
  "text": "outdoor",
  "sources": []
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/customer/attribution`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### GET `/v2/customer/members`

List the people on the account.

##### Query parameters

| Name | Type | Description |
|---|---|---|
| `limit` | integer | How many items to return, from 1 to 200. Defaults to 50. |
| `cursor` | string | An opaque position from an earlier response's `next_cursor`. |

##### Example request

```bash
curl "https://api.example.com/v2/customer/members" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `members` | array | Each with `name`, `email` and `role`. |

###### Example

```json
{
  "members": []
}
```

Responses carry an `ETag`, and a request with a matching `If-None-Match` gets a 304 with no body.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/customer/members`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

#### POST `/v2/customer/members`

Invite somebody to the account.

##### Parameters

| Name | Type | Description |
|---|---|---|
| `email` | string | Their address, such as `someone@example.org`. |
| `role` | string | `admin`, `billing` or `viewer`. |

##### Example request

```bash
curl -X POST "https://api.example.com/v2/customer/members" \
  -H "Authorization: Bearer $FERNWAY_KEY"
```

##### Response

| Field | Type | Description |
|---|---|---|
| `invited` | boolean | Always true. |
| `expires_at` | timestamp | When the invitation stops working. |

###### Example

```json
{
  "invited": true,
  "expires_at": "2026-03-02T09:30:00Z"
}
```

Send an `Idempotency-Key` header to make a retry safe. Keys are remembered for 24 hours.

##### Errors

| Status | Error | When |
|---|---|---|
| `401` | `invalid_key` | The key is missing or revoked. |
| `404` | `not_found` | Nothing at `/v2/customer/members`. |
| `429` | `rate_limited` | Over the limit; see `Retry-After`. |

## Changelog

| Date | Change |
|---|---|
| 2026-02-20 | `/v2/route/profile` returns `ascent_m`. |
| 2025-11-03 | Webhook deliveries are signed with SHA-256; the old scheme stops in June. |
| 2025-06-17 | Vector tiles are gzip-encoded by default. |
| 2024-09-30 | Version 2. Version 1's tile URLs keep working, and everything else redirects. |
