"use strict"

const { test } = require("node:test")
const assert = require("node:assert/strict")
const Model = require("../Model.js")

function output(overrides = {}) {
  return JSON.stringify({
    generated_at: "2026-09-05T08:00:00Z",
    data_updated_at: "2026-09-05T07:57:00Z",
    stale: true,
    departures: [{
      departure: {
        line: " 158 ",
        headsign: " Letňany ",
        platform_code: " A "
      },
      boarding_point_name: " Nové Letňany ",
      leave_in_seconds: 270,
      departs_in_seconds: 630
    }],
    ...overrides
  })
}

test("settings are parsed and bounded defensively", () => {
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
})

test("command errors strip only the exact CLI prefix", () => {
  assert.equal(Model.commandError("pidjezdy: request failed\nmore detail", 1), "request failed")
  assert.equal(Model.commandError("another command failed", 1), "another command failed")
  assert.equal(Model.commandError("", 7), "pidjezdy exited with status 7")
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
    leaveSeconds: 270,
    departsSeconds: 630
  })
})

test("parseOutput rejects malformed envelopes and records", () => {
  assert.equal(Model.parseOutput("not json").ok, false)
  assert.equal(Model.parseOutput("{}").ok, false)
  assert.equal(Model.parseOutput(output({ generated_at: "never" })).ok, false)
  assert.equal(Model.parseOutput(output({ departures: [{}] })).ok, false)
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
})

test("countdown labels round down conservatively", () => {
  assert.equal(Model.leaveLabel(299), "leave in 4 min")
  assert.equal(Model.leaveLabel(59), "leave now")
  assert.equal(Model.departureLabel(659), "departs in 10 min")
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
