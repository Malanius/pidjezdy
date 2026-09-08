# PID departure-board API notes

`pidjezdy` reads departures from the endpoint used by PID's public departure
board:

```text
https://data.pid.cz/departures/data.php
```

This is an observed contract, not an official API specification. The notes
below were verified against the public endpoint and its
[`departure-board.js` frontend][frontend] on 2026-09-02 and 2026-09-04. The
service can change without notice.

## Request

The endpoint accepts an HTTP `GET` request. `pidjezdy` sends these query
parameters explicitly:

| Parameter | Shape | Observed behavior |
| --- | --- | --- |
| `stopIds[]` | Repeated JSON value | Each value has the form `{"0":["PLATFORM_ID", ...]}`. One request may contain multiple values. |
| `minutesAfter` | Positive integer | Future time window in minutes. The web frontend defaults to `120`; an omitted value appeared to use about `60`. |
| `limit` | Integer from 1 to 20 | Maximum departures returned for each `stopIds[]` group. The observed server default is `5`; values above `20` return an API error. |

For example, this requests at most three departures for each of two boarding
point groups:

```console
curl --get 'https://data.pid.cz/departures/data.php' \
  --data-urlencode 'minutesAfter=120' \
  --data-urlencode 'limit=3' \
  --data-urlencode 'stopIds[]={"0":["U100Z1P","U100Z2P"]}' \
  --data-urlencode 'stopIds[]={"0":["U200Z1P"]}'
```

The identifiers in this example are placeholders.

The frontend also knows about the following parameters. `pidjezdy` currently
omits them:

- `minutesBefore` defaults to `0` and is omitted by the frontend at that
  value.
- `offset` defaults to `0` and is optional.
- `filter` accepts `none`, `routeOnce`, `routeHeadingOnce`,
  `routeOnceFill`, `routeHeadingOnceFill`, `routeHeadingOnceNoGap`, and
  `routeHeadingOnceNoGapFill`. `pidjezdy` requests unfiltered data and applies
  its configured line and direction rules locally.
- The frontend removes `mode` for this public endpoint. These notes therefore
  cover departures only.

Direct probes found two details that are important for multi-stop requests:

- Repeating `stopIds[]` returns one independently limited array per group, so
  the configured boarding points can be fetched in one request.
- Response groups did not consistently follow request order. Consumers must
  associate departures by `stop.id`, not by the outer array index.

A JSON object with multiple numeric keys in one `stopIds[]` value returned an
error. The client consequently emits one `{"0":[...]}` object per group.

## Successful response

A successful response is a nested array. The outer level represents requested
groups, while every inner array contains departure objects. The client
deliberately flattens these arrays and relies on each departure's stop data.

Fields consumed by `pidjezdy` are:

| JSON path | Meaning and handling |
| --- | --- |
| `departure.timestamp_scheduled` | Required ISO 8601 timestamp with an offset; normalized to UTC. |
| `departure.timestamp_predicted` | Nullable predicted timestamp; normalized to UTC when present. |
| `departure.delay_seconds` | Nullable delay in seconds. |
| `stop.id` | Boarding-point/platform identifier used to match configuration. |
| `stop.platform_code` | Nullable passenger-facing platform code. |
| `route.short_name` | Public line number or name. |
| `trip.id` | Trip identifier. |
| `trip.headsign` | Destination/direction label. |
| `trip.is_canceled` | Cancellation flag, normalized to `is_cancelled`; cancelled records remain otherwise structurally identical to regular departures. |
| `vehicle` | Nullable vehicle object; `id`, `is_wheelchair_accessible`, `is_air_conditioned`, and `has_charger` are retained when supplied. |

Surrounding whitespace is removed from the textual identifiers, line,
headsign, platform code, and vehicle ID before the provider model is passed to
the core selector.

The response also contains a server-calculated `minutes` value and fields not
used by this project. Unknown fields are intentionally tolerated. `pidjezdy`
calculates time-to-departure from the timestamps so ranking uses a consistent
clock.

## Errors

Validation failures can arrive with HTTP status `200` and this JSON shape:

```json
{
  "error_message": "...",
  "error_status": 400,
  "error_info": "..."
}
```

For example, `limit=50` produced a structured error with `error_status: 400`.
The client recognizes this separately from malformed JSON. It also treats an
actual non-successful HTTP status as an error and retains a textual
representation of the response body for diagnostics.

## Transport behavior

The frontend refresh interval is 15 seconds and its HTTP timeout is 45
seconds. Responses were observed with `Cache-Control: max-age=30`. These are
observations, not service guarantees.

The server was also observed sending `Content-Encoding: gzip` inconsistently:
the response body may either be gzip bytes or already-decoded JSON. This makes
strict header-driven clients fail in some cases (`curl --compressed` can report
error 61). `pidjezdy` requests identity encoding, disables automatic gzip
decoding, and inspects the gzip magic bytes before decoding. Its request
timeout is 15 seconds and it does not retry; caching and stale-data behavior
belong to a later integration layer.

## Maintenance probes

Keep probes small and use non-sensitive boarding-point identifiers:

```console
curl -i --get 'https://data.pid.cz/departures/data.php' \
  --data-urlencode 'minutesAfter=10' \
  --data-urlencode 'limit=1' \
  --data-urlencode 'stopIds[]={"0":["PLATFORM_ID"]}'
```

When the endpoint behavior changes, update the sanitized fixtures in
`crates/pidjezdy-pid/fixtures/`, the parser/client tests, and this
document together.

[frontend]: https://data.pid.cz/departures/lib/departure-board.js
