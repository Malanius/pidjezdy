"use strict"

const { test } = require("node:test")
const assert = require("node:assert/strict")
const Model = require("../Model.js")

function output(overrides = {}) {
  return JSON.stringify({
    schema_version: 2,
    generated_at: "2026-09-05T08:00:00Z",
    data_updated_at: "2026-09-05T07:57:00Z",
    stale: true,
    departures: [{
      departure: {
        line: " 158 ",
        headsign: " Letňany ",
        platform_code: " A ",
        delay_seconds: 125
      },
      boarding_point_name: " Nové Letňany ",
      leave_in_seconds: 270,
      departs_in_seconds: 630
    }],
    cancelled: [],
    ...overrides
  })
}

test("settings are parsed and bounded defensively", () => {
  assert.equal(Model.maxDisplayDepartures, 20)
  assert.equal(Model.refreshInterval(undefined), 60)
  assert.equal(Model.refreshInterval(null), 60)
  assert.equal(Model.refreshInterval(""), 60)
  assert.equal(Model.refreshInterval("  "), 60)
  assert.equal(Model.refreshInterval(false), 60)
  assert.equal(Model.refreshInterval([]), 60)
  assert.equal(Model.refreshInterval(5), 30)
  assert.equal(Model.refreshInterval("120"), 120)
  assert.equal(Model.departureLimit(undefined), 3)
  assert.equal(Model.departureLimit(null), 3)
  assert.equal(Model.departureLimit(""), 3)
  assert.equal(Model.departureLimit({}), 3)
  assert.equal(Model.departureLimit(50), 20)
  assert.equal(Model.binaryPath(undefined), "pidjezdy")
  assert.equal(Model.binaryPath(""), "pidjezdy")
  assert.equal(Model.binaryPath("  "), "pidjezdy")
  assert.equal(Model.binaryPath(" /opt/pidjezdy/bin/pidjezdy "), "/opt/pidjezdy/bin/pidjezdy")
})

test("process errors distinguish failed launches from failed runs", () => {
  assert.equal(Model.exitError(7), "pidjezdy exited with status 7")
  assert.equal(
    Model.commandError("/missing/pidjezdy"),
    "could not run /missing/pidjezdy — check the plugin's binary path setting"
  )
  assert.equal(
    Model.stderrError("invalid configuration\ncaused by: missing stops", 2),
    "invalid configuration"
  )
  assert.equal(Model.stderrError("pidjezdy: invalid configuration", 2), "invalid configuration")
  assert.equal(Model.stderrError("  ", 2), "pidjezdy exited with status 2")
})

test("parseOutput validates and flattens the CLI envelope", () => {
  const report = Model.parseOutput(output())
  assert.equal(report.ok, true)
  assert.equal(report.stale, true)
  assert.deepEqual(report.departures[0], {
    line: "158",
    headsign: "Letňany",
    boardingPoint: "Nové Letňany",
    platform: "A",
    delaySeconds: 125,
    leaveSeconds: 270,
    departsSeconds: 630
  })
})

test("parseOutput rejects malformed envelopes and records", () => {
  assert.equal(Model.parseOutput("not json").ok, false)
  assert.equal(Model.parseOutput("{}").ok, false)
  assert.equal(Model.parseOutput(output({ generated_at: "never" })).ok, false)
  assert.equal(Model.parseOutput(output({ departures: [{}] })).ok, false)
  assert.equal(
    Model.parseOutput(output({ cancelled: [{}] })).error,
    "pidjezdy returned an unsupported cancellation record"
  )
  assert.equal(
    Model.parseOutput(output({ cancelled: [{ departure: null }] })).error,
    "pidjezdy returned an unsupported cancellation record"
  )
})

test("parseOutput requires the matching envelope version", () => {
  assert.equal(Model.expectedSchemaVersion, 2)
  assert.equal(
    Model.parseOutput(output({ schema_version: 1 })).error,
    "pidjezdy speaks envelope v1; this plugin needs v2 — update the plugin"
  )
  assert.equal(
    Model.parseOutput(output({ schema_version: undefined })).error,
    "pidjezdy speaks envelope vundefined; this plugin needs v2 — update the plugin"
  )
})

test("parseOutput validates and exposes CLI error envelopes", () => {
  var report = Model.parseOutput(JSON.stringify({
    schema_version: 2,
    generated_at: "2026-09-05T08:00:00Z",
    error: {
      kind: "departures_unavailable",
      message: "departures unavailable",
      causes: ["connection refused"]
    }
  }))
  assert.deepEqual(report, {
    ok: false,
    commandError: true,
    errorKind: "departures_unavailable",
    error: "departures unavailable",
    causes: ["connection refused"],
    generatedAtMs: Date.parse("2026-09-05T08:00:00Z")
  })

  assert.equal(Model.parseOutput(JSON.stringify({
    schema_version: 2,
    generated_at: "2026-09-05T08:00:00Z",
    error: { kind: "fetch_failed", message: "failed", causes: [7] }
  })).error, "pidjezdy returned an unsupported error document")
})

test("error summary adds only the root cause", () => {
  var report = {
    error: "departures unavailable",
    causes: [
      "cached fallback unavailable at /home/alice/departures.json",
      "could not fetch PID departures",
      "connection refused"
    ]
  }
  assert.equal(
    Model.errorSummary(report),
    "departures unavailable — connection refused"
  )
  assert.equal(Model.errorSummary({ error: "configuration invalid", causes: [] }), "configuration invalid")
  assert.equal(Model.errorSummary(null), "")
})

test("tooltip keeps detailed causes and filesystem paths out", () => {
  var report = {
    error: "departures unavailable",
    causes: ["no cached departures at /home/alice/departures.json"]
  }
  var tooltip = Model.tooltip(null, Date.now(), report.error, false)

  assert.equal(
    Model.errorSummary(report),
    "departures unavailable — no cached departures at /home/alice/departures.json"
  )
  assert.equal(tooltip, "PID departures · departures unavailable")
  assert.equal(tooltip.includes("/home/alice"), false)
})

test("currentDepartures advances countdowns and removes missed options", () => {
  const report = Model.parseOutput(output({
    departures: [
      {
        departure: { line: "158", headsign: "Letňany", platform_code: "A" },
        boarding_point_name: "Near",
        leave_in_seconds: 30,
        departs_in_seconds: 390
      },
      {
        departure: { line: "195", headsign: "Town", platform_code: null },
        boarding_point_name: "Far",
        leave_in_seconds: 120,
        departs_in_seconds: 600
      }
    ]
  }))
  const rows = Model.currentDepartures(report, Date.parse("2026-09-05T08:01:00Z"))
  assert.equal(rows.length, 1)
  assert.equal(rows[0].line, "195")
  assert.equal(rows[0].leaveSeconds, 60)
  assert.equal(rows[0].departsSeconds, 540)
  assert.equal(rows[0].delaySeconds, null)
})

test("cancellations are validated, advanced, and removed after departure", () => {
  const report = Model.parseOutput(output({
    departures: [],
    cancelled: [{
      departure: {
        line: "158",
        headsign: "Letňany",
        platform_code: "A",
        is_cancelled: true
      },
      boarding_point_name: "Near",
      leave_in_seconds: 30,
      departs_in_seconds: 90
    }]
  }))

  assert.equal(report.ok, true)
  const current = Model.currentCancellations(report, Date.parse("2026-09-05T08:01:00Z"))
  assert.equal(current.length, 1)
  assert.equal(current[0].departsSeconds, 30)
  assert.equal(
    Model.currentCancellations(report, Date.parse("2026-09-05T08:01:31Z")).length,
    0
  )
})

test("countdown labels round down conservatively", () => {
  assert.equal(Model.leaveLabel(299), "leave in 4 min")
  assert.equal(Model.leaveLabel(59), "leave now")
  assert.equal(Model.departureLabel(659), "departs in 10 min")
  assert.equal(Model.cancellationLabel(659), "would have departed in 10 min")
})

test("empty state distinguishes cancellation notes from no cancellation data", () => {
  assert.equal(Model.emptyStateLabel(0), "No reachable configured departures.")
  assert.equal(Model.emptyStateLabel(undefined), "No reachable configured departures.")
  assert.equal(Model.emptyStateLabel(NaN), "No reachable configured departures.")
  assert.equal(
    Model.emptyStateLabel("1"),
    "No reachable departures."
  )
})

test("delay labels only material late and early running", () => {
  for (const [delay, expected] of [
    [null, ""],
    [0, ""],
    [59, ""],
    [60, "+1 late"],
    [125, "+2 late"],
    [-59, ""],
    [-60, "-1 early"],
    [-125, "-2 early"]
  ]) {
    assert.equal(Model.delayLabel(delay), expected)
  }
  assert.equal(
    Model.departureTimingLabel(125, 659),
    "+2 late, departs in 10 min"
  )
  assert.equal(Model.departureTimingLabel(null, 659), "departs in 10 min")
})

test("stale age uses human scale boundaries", () => {
  for (const [seconds, expected] of [
    [0, "just now"],
    [59, "just now"],
    [60, "1 min ago"],
    [3599, "59 min ago"],
    [3600, "1h 0m ago"],
    [7800, "2h 10m ago"],
    [86399, "23h 59m ago"],
    [86400, "1d ago"]
  ]) {
    assert.equal(Model.staleAgeLabel(seconds), expected)
  }
})

test("tooltip makes an all-cancelled result explicit", () => {
  const report = Model.parseOutput(output({
    departures: [],
    cancelled: [{
      departure: {
        line: "158",
        headsign: "Letňany",
        platform_code: "A",
        is_cancelled: true
      },
      boarding_point_name: "Near",
      leave_in_seconds: 270,
      departs_in_seconds: 630
    }]
  }))

  assert.equal(
    Model.tooltip(report, Date.parse("2026-09-05T08:00:30Z"), "", false),
    "STALE · 158 → Letňany · cancelled"
  )
})

test("tooltip describes the nearest current departure and stale state", () => {
  const report = Model.parseOutput(output())
  const now = Date.parse("2026-09-05T08:00:30Z")
  assert.equal(Model.tooltip(report, now, "", false), "STALE · 158 → Letňany · leave in 4 min")
  assert.equal(Model.updateLabel(report, now), "STALE · updated 3 min ago")
  assert.equal(Model.tooltip(null, now, "command failed", false), "PID departures · command failed")
})

test("tooltip marks retained departures when the latest refresh failed", () => {
  const report = Model.parseOutput(output({ stale: false }))
  const now = Date.parse("2026-09-05T08:00:30Z")

  assert.equal(
    Model.tooltip(report, now, "request failed", false),
    "⚠ 158 → Letňany · leave in 4 min"
  )
})
