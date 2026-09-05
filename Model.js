// Pure presentation helpers for the Omarchy plugin. The CLI remains
// responsible for fetching, matching, reachability, quotas, and deduplication;
// this adapter validates its JSON envelope and keeps countdowns honest between
// polls. No Qt imports are used so the behavior stays testable with Node.

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
  return boundedInteger(value, 3, 1, 20)
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
  if (line === "" || headsign === "" || boardingPoint === ""
      || leaveSeconds === null || departsSeconds === null) return null
  return {
    line: line,
    headsign: headsign,
    boardingPoint: boardingPoint,
    platform: nonEmptyString(raw.platform_code),
    leaveSeconds: Math.floor(leaveSeconds),
    departsSeconds: Math.floor(departsSeconds)
  }
}

function parseOutput(raw) {
  try {
    var document = JSON.parse(String(raw || ""))
    if (!document || typeof document !== "object" || !Array.isArray(document.departures))
      return { ok: false, error: "pidjezdy returned an unsupported JSON document" }

    var generatedAtMs = Date.parse(String(document.generated_at || ""))
    var dataUpdatedAtMs = Date.parse(String(document.data_updated_at || ""))
    if (!isFinite(generatedAtMs) || !isFinite(dataUpdatedAtMs))
      return { ok: false, error: "pidjezdy returned invalid timestamps" }

    var departures = []
    for (var i = 0; i < document.departures.length; i++) {
      var departure = normalizeDeparture(document.departures[i])
      if (!departure)
        return { ok: false, error: "pidjezdy returned an unsupported departure record" }
      departures.push(departure)
    }
    return {
      ok: true,
      generatedAtMs: generatedAtMs,
      dataUpdatedAtMs: dataUpdatedAtMs,
      stale: document.stale === true,
      departures: departures
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
      leaveSeconds: leaveSeconds,
      departsSeconds: row.departsSeconds - elapsed
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

function updateLabel(report, nowMs) {
  if (!report || !isFinite(report.dataUpdatedAtMs)) return ""
  var ageMinutes = wholeMinutes((Number(nowMs) - report.dataUpdatedAtMs) / 1000)
  return (report.stale ? "STALE · " : "") + "updated " + ageMinutes + " min ago"
}

function commandError(stderrText, exitCode) {
  var prefix = "pidjezdy: "
  var message = String(stderrText || "").trim().split("\n")[0]
  if (message.indexOf(prefix) === 0) message = message.substring(prefix.length)
  return message || "pidjezdy exited with status " + exitCode
}

function tooltip(report, nowMs, errorMessage, loading) {
  var rows = currentDepartures(report, nowMs)
  if (rows.length > 0) {
    var first = rows[0]
    var prefix = errorMessage ? "⚠ " : ""
    if (report && report.stale) prefix += "STALE · "
    return prefix + first.line + " → " + first.headsign + " · " + leaveLabel(first.leaveSeconds)
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
    normalizeDeparture: normalizeDeparture,
    parseOutput: parseOutput,
    elapsedSeconds: elapsedSeconds,
    currentDepartures: currentDepartures,
    wholeMinutes: wholeMinutes,
    leaveLabel: leaveLabel,
    departureLabel: departureLabel,
    updateLabel: updateLabel,
    commandError: commandError,
    tooltip: tooltip
  }
}
