// Pure presentation helpers for the Omarchy plugin. The CLI remains
// responsible for fetching, matching, reachability, quotas, and deduplication;
// this adapter validates its JSON envelope and keeps countdowns honest between
// polls. No Qt imports are used so the behavior stays testable with Node.

var EXPECTED_SCHEMA_VERSION = 2
// Keep this and manifest.json's limit maximum aligned with
// pidjezdy_core::config::MAX_DISPLAY_DEPARTURES. CI verifies the contract.
var MAX_DISPLAY_DEPARTURES = 20

function boundedInteger(value, fallback, minimum, maximum) {
  var numeric = typeof value === "number"
    || (typeof value === "string" && value.trim() !== "")
  var parsed = numeric ? Math.floor(Number(value)) : NaN
  if (!isFinite(parsed)) parsed = fallback
  return Math.max(minimum, Math.min(maximum, parsed))
}

function refreshInterval(value) {
  return boundedInteger(value, 60, 30, 3600)
}

function departureLimit(value) {
  return boundedInteger(value, 3, 1, MAX_DISPLAY_DEPARTURES)
}

function binaryPath(value) {
  return nonEmptyString(value) || "pidjezdy"
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim() !== "" ? value.trim() : ""
}

function finiteNumber(value) {
  var number = Number(value)
  return isFinite(number) ? number : null
}

function normalizeDeparture(value) {
  if (!value || typeof value !== "object" || !value.departure) return null
  var raw = value.departure
  var line = nonEmptyString(raw.line)
  var headsign = nonEmptyString(raw.headsign)
  var boardingPoint = nonEmptyString(value.boarding_point_name)
  var leaveSeconds = finiteNumber(value.leave_in_seconds)
  var departsSeconds = finiteNumber(value.departs_in_seconds)
  var delaySeconds = null
  if (raw.delay_seconds !== null && raw.delay_seconds !== undefined) {
    delaySeconds = finiteNumber(raw.delay_seconds)
    if (delaySeconds === null) return null
    delaySeconds = delaySeconds < 0 ? Math.ceil(delaySeconds) : Math.floor(delaySeconds)
  }
  if (line === "" || headsign === "" || boardingPoint === ""
      || leaveSeconds === null || departsSeconds === null) return null
  return {
    line: line,
    headsign: headsign,
    boardingPoint: boardingPoint,
    platform: nonEmptyString(raw.platform_code),
    delaySeconds: delaySeconds,
    leaveSeconds: Math.floor(leaveSeconds),
    departsSeconds: Math.floor(departsSeconds)
  }
}

function normalizeCancellation(value) {
  if (!value || typeof value !== "object" || !value.departure
      || value.departure.is_cancelled !== true) return null
  return normalizeDeparture(value)
}

function parseOutput(raw) {
  try {
    var document = JSON.parse(String(raw || ""))
    if (!document || typeof document !== "object" || Array.isArray(document))
      return { ok: false, error: "pidjezdy returned an unsupported JSON document" }
    if (document.schema_version !== EXPECTED_SCHEMA_VERSION)
      return {
        ok: false,
        error: "pidjezdy speaks envelope v" + String(document.schema_version)
          + "; this plugin needs v" + EXPECTED_SCHEMA_VERSION + " — update the plugin"
      }

    var generatedAtMs = Date.parse(String(document.generated_at || ""))
    if (!isFinite(generatedAtMs))
      return { ok: false, error: "pidjezdy returned invalid timestamps" }

    if (document.error !== undefined) {
      var rawError = document.error
      if (!rawError || typeof rawError !== "object" || Array.isArray(rawError)
          || nonEmptyString(rawError.kind) === "" || nonEmptyString(rawError.message) === ""
          || !Array.isArray(rawError.causes))
        return { ok: false, error: "pidjezdy returned an unsupported error document" }
      var causes = []
      for (var causeIndex = 0; causeIndex < rawError.causes.length; causeIndex++) {
        var cause = nonEmptyString(rawError.causes[causeIndex])
        if (cause === "")
          return { ok: false, error: "pidjezdy returned an unsupported error document" }
        causes.push(cause)
      }
      return {
        ok: false,
        commandError: true,
        errorKind: nonEmptyString(rawError.kind),
        error: nonEmptyString(rawError.message),
        causes: causes,
        generatedAtMs: generatedAtMs
      }
    }

    if (!Array.isArray(document.departures) || !Array.isArray(document.cancelled))
      return { ok: false, error: "pidjezdy returned an unsupported JSON document" }
    var dataUpdatedAtMs = Date.parse(String(document.data_updated_at || ""))
    if (!isFinite(dataUpdatedAtMs))
      return { ok: false, error: "pidjezdy returned invalid timestamps" }

    var departures = []
    for (var i = 0; i < document.departures.length; i++) {
      var departure = normalizeDeparture(document.departures[i])
      if (!departure)
        return { ok: false, error: "pidjezdy returned an unsupported departure record" }
      departures.push(departure)
    }
    var cancelled = []
    for (var cancelledIndex = 0; cancelledIndex < document.cancelled.length; cancelledIndex++) {
      var cancellation = normalizeCancellation(document.cancelled[cancelledIndex])
      if (!cancellation)
        return { ok: false, error: "pidjezdy returned an unsupported cancellation record" }
      cancelled.push(cancellation)
    }
    return {
      ok: true,
      generatedAtMs: generatedAtMs,
      dataUpdatedAtMs: dataUpdatedAtMs,
      stale: document.stale === true,
      departures: departures,
      cancelled: cancelled
    }
  } catch (error) {
    return { ok: false, error: "pidjezdy returned malformed JSON" }
  }
}

function elapsedSeconds(report, nowMs) {
  if (!report || !isFinite(report.generatedAtMs)) return 0
  return Math.max(0, Math.floor((Number(nowMs) - report.generatedAtMs) / 1000))
}

function currentDepartures(report, nowMs) {
  if (!report || !Array.isArray(report.departures)) return []
  var elapsed = elapsedSeconds(report, nowMs)
  var current = []
  for (var i = 0; i < report.departures.length; i++) {
    var row = report.departures[i]
    var leaveSeconds = row.leaveSeconds - elapsed
    if (leaveSeconds < 0) continue
    current.push({
      line: row.line,
      headsign: row.headsign,
      boardingPoint: row.boardingPoint,
      platform: row.platform,
      delaySeconds: row.delaySeconds,
      leaveSeconds: leaveSeconds,
      departsSeconds: row.departsSeconds - elapsed
    })
  }
  return current
}

function currentCancellations(report, nowMs) {
  if (!report || !Array.isArray(report.cancelled)) return []
  var elapsed = elapsedSeconds(report, nowMs)
  var current = []
  for (var i = 0; i < report.cancelled.length; i++) {
    var row = report.cancelled[i]
    var departsSeconds = row.departsSeconds - elapsed
    if (departsSeconds < 0) continue
    current.push({
      line: row.line,
      headsign: row.headsign,
      boardingPoint: row.boardingPoint,
      platform: row.platform,
      delaySeconds: row.delaySeconds,
      leaveSeconds: row.leaveSeconds - elapsed,
      departsSeconds: departsSeconds
    })
  }
  return current
}

function wholeMinutes(seconds) {
  return Math.floor(Math.max(0, Number(seconds) || 0) / 60)
}

function leaveLabel(seconds) {
  var minutes = wholeMinutes(seconds)
  return minutes === 0 ? "leave now" : "leave in " + minutes + " min"
}

function departureLabel(seconds) {
  return "departs in " + wholeMinutes(seconds) + " min"
}

function delayLabel(delaySeconds) {
  if (delaySeconds === null || delaySeconds === undefined || !isFinite(Number(delaySeconds)))
    return ""
  var delay = Number(delaySeconds)
  if (delay >= 60) return "+" + Math.floor(delay / 60) + " late"
  if (delay <= -60) return "-" + Math.floor(Math.abs(delay) / 60) + " early"
  return ""
}

function departureTimingLabel(delaySeconds, departsSeconds) {
  var delay = delayLabel(delaySeconds)
  return (delay ? delay + ", " : "") + departureLabel(departsSeconds)
}

function cancellationLabel(seconds) {
  return "would have departed in " + wholeMinutes(seconds) + " min"
}

function staleAgeLabel(seconds) {
  var ageSeconds = Math.floor(Math.max(0, Number(seconds) || 0))
  if (ageSeconds < 60) return "just now"
  if (ageSeconds < 3600) return Math.floor(ageSeconds / 60) + " min ago"
  if (ageSeconds < 86400)
    return Math.floor(ageSeconds / 3600) + "h "
      + Math.floor(ageSeconds % 3600 / 60) + "m ago"
  return Math.floor(ageSeconds / 86400) + "d ago"
}

function updateLabel(report, nowMs) {
  if (!report || !isFinite(report.dataUpdatedAtMs)) return ""
  var ageSeconds = (Number(nowMs) - report.dataUpdatedAtMs) / 1000
  return (report.stale ? "STALE · " : "") + "updated " + staleAgeLabel(ageSeconds)
}

function exitError(exitCode) {
  return "pidjezdy exited with status " + exitCode
}

function commandError(binaryPath) {
  return "could not run " + binaryPath + " — check the plugin's binary path setting"
}

function stderrError(stderr, exitCode) {
  var diagnostic = nonEmptyString(stderr).split(/\r?\n/)[0]
  return diagnostic || exitError(exitCode)
}

function tooltip(report, nowMs, errorMessage, loading) {
  var rows = currentDepartures(report, nowMs)
  var cancellations = currentCancellations(report, nowMs)
  if (rows.length > 0) {
    var first = rows[0]
    var prefix = errorMessage ? "⚠ " : ""
    if (report && report.stale) prefix += "STALE · "
    return prefix + first.line + " → " + first.headsign + " · " + leaveLabel(first.leaveSeconds)
  }
  if (cancellations.length > 0) {
    var cancellation = cancellations[0]
    var cancellationPrefix = errorMessage ? "⚠ " : ""
    if (report && report.stale) cancellationPrefix += "STALE · "
    return cancellationPrefix + cancellation.line + " → " + cancellation.headsign + " · cancelled"
  }
  if (errorMessage) return "PID departures · " + errorMessage
  if (loading) return "PID departures · updating…"
  return "PID departures · no reachable departures"
}

if (typeof module !== "undefined" && module && module.exports) {
  module.exports = {
    boundedInteger: boundedInteger,
    refreshInterval: refreshInterval,
    departureLimit: departureLimit,
    binaryPath: binaryPath,
    maxDisplayDepartures: MAX_DISPLAY_DEPARTURES,
    expectedSchemaVersion: EXPECTED_SCHEMA_VERSION,
    normalizeDeparture: normalizeDeparture,
    normalizeCancellation: normalizeCancellation,
    parseOutput: parseOutput,
    elapsedSeconds: elapsedSeconds,
    currentDepartures: currentDepartures,
    currentCancellations: currentCancellations,
    wholeMinutes: wholeMinutes,
    leaveLabel: leaveLabel,
    departureLabel: departureLabel,
    delayLabel: delayLabel,
    departureTimingLabel: departureTimingLabel,
    cancellationLabel: cancellationLabel,
    staleAgeLabel: staleAgeLabel,
    updateLabel: updateLabel,
    exitError: exitError,
    commandError: commandError,
    stderrError: stderrError,
    tooltip: tooltip
  }
}
