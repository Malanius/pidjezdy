import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui
import "Model.js" as Model

// Periodically asks the pidjezdy CLI for already-filtered, actionable
// departures. QML owns only polling and presentation; PID API details and all
// selection policy remain in the reusable Rust core.
Panel {
  id: root
  moduleName: "malanius.pidjezdy"
  ipcTarget: "malanius.pidjezdy"
  manageIpc: false

  readonly property string glyph: "󰃧"
  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property color urgent: bar ? bar.urgent : Color.urgent
  readonly property color dim: Qt.darker(foreground, 1.45)
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family
  readonly property string binaryPath: Model.binaryPath(setting("binaryPath", "pidjezdy"))
  readonly property int refreshIntervalSec: Model.refreshInterval(setting("refreshIntervalSec", 60))
  readonly property int requestedLimit: Model.departureLimit(setting("limit", 3))

  property bool ready: false
  property bool refreshQueued: false
  property string stdoutText: ""
  property string stderrText: ""
  property string errorMessage: ""
  property string errorDetail: ""
  property var report: null
  property double nowMs: Date.now()
  property double lastAttemptMs: 0

  readonly property var departures: Model.currentDepartures(report, nowMs)
  readonly property var cancellations: Model.currentCancellations(report, nowMs)
  readonly property bool stale: report ? report.stale === true : false
  readonly property bool loading: queryProcess.running

  visible: true
  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  function refresh() {
    if (queryProcess.running) {
      refreshQueued = true
      return
    }
    stdoutText = ""
    stderrText = ""
    queryProcess.command = [
      root.binaryPath, "departures",
      "--limit", String(requestedLimit),
      "--format", "json"
    ]
    queryProcess.running = true
  }

  function finishQuery(exitCode) {
    lastAttemptMs = Date.now()
    nowMs = lastAttemptMs
    refreshTimer.restart()
    var parsed = Model.parseOutput(stdoutText)
    var message = ""
    var detail = ""
    if (parsed.ok && exitCode === 0) {
      report = parsed
    } else if (parsed.commandError === true) {
      message = parsed.error
      detail = Model.errorSummary(parsed)
    } else if (exitCode !== 0 && String(stdoutText || "").trim() === "") {
      message = String(stderrText || "").trim() === ""
        ? Model.commandError(root.binaryPath)
        : Model.stderrError(stderrText, exitCode)
    } else {
      message = parsed.ok ? Model.exitError(exitCode) : parsed.error
    }
    errorMessage = message
    errorDetail = detail || message

    if (refreshQueued) {
      refreshQueued = false
      Qt.callLater(root.refresh)
    }
  }

  function open() {
    root.controller.show()
    nowMs = Date.now()
    var minimumAgeMs = Model.openRefreshMinimumAge(
      report !== null,
      errorMessage,
      refreshIntervalSec * 1000
    )
    if (Model.shouldRefreshOnOpen(
        queryProcess.running,
        lastAttemptMs,
        nowMs,
        minimumAgeMs
    )) refresh()
    Qt.callLater(function() { keyCatcher.forceActiveFocus() })
  }

  function close() { root.controller.hide() }
  function toggle() { opened ? close() : open() }

  onRefreshIntervalSecChanged: if (ready) refreshTimer.restart()
  onRequestedLimitChanged: if (ready) refreshDebounce.restart()
  onBinaryPathChanged: if (ready) refreshDebounce.restart()

  Component.onCompleted: {
    ready = true
    refresh()
  }

  Timer {
    id: refreshTimer
    interval: root.refreshIntervalSec * 1000
    running: root.ready
    repeat: true
    onTriggered: root.refresh()
  }

  Timer {
    id: refreshDebounce
    interval: 100
    repeat: false
    onTriggered: root.refresh()
  }

  // The panel updates exact second-based countdowns locally between network
  // polls, dropping a departure as soon as its leave-by time passes.
  Timer {
    interval: 1000
    running: root.opened
    repeat: true
    onTriggered: root.nowMs = Date.now()
  }

  Process {
    id: queryProcess
    running: false

    onExited: function(exitCode) {
      // Let waitForEnd collectors publish their final text first.
      Qt.callLater(function() { root.finishQuery(exitCode) })
    }

    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.stdoutText = text
    }

    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.stderrText = text
    }
  }

  IpcHandler {
    target: root.ipcTarget
    function open(): void { root.open() }
    function close(): void { root.close() }
    function show(): void { root.open() }
    function hide(): void { root.close() }
    function toggle(): void { root.toggle() }
    function refresh(): string { root.refresh(); return "ok" }
    function status(): string {
      return JSON.stringify({
        loading: root.loading,
        stale: root.stale,
        error: root.errorMessage,
        departures: root.departures.length,
        cancelled: root.cancellations.length,
        updated: Model.updateLabel(root.report, root.nowMs)
      })
    }
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.glyph
    active: root.stale || root.errorMessage !== "" || root.cancellations.length > 0
    tooltipText: Model.tooltip(root.report, root.nowMs, root.errorMessage, root.loading)

    onPressed: function(buttonCode) {
      if (buttonCode === Qt.MiddleButton) root.refresh()
      else root.toggle()
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(420))
    contentHeight: panel.fittedContentHeight(panelColumn.implicitHeight, Style.space(600))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent

      onMoveRequested: function(dx, dy) {
        if (dy === 0) return
        panelScroll.contentY = Math.max(0, Math.min(
          panelScroll.contentHeight - panelScroll.height,
          panelScroll.contentY + dy * Style.space(52)
        ))
      }
      onActivateRequested: root.refresh()
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }
      onTextKey: function(text) {
        if (text === "r" || text === "R") root.refresh()
      }

      Flickable {
        id: panelScroll
        anchors.fill: parent
        contentWidth: width
        contentHeight: panelColumn.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        interactive: contentHeight > height
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

        Column {
          id: panelColumn
          width: panelScroll.width
          spacing: Style.space(12)

          PanelHero {
            width: parent.width
            title: "PID Departures"
            meta: root.loading && !root.report ? "Updating" : Model.updateLabel(root.report, root.nowMs)
            detail: root.cancellations.length > 0
              ? root.departures.length + " · " + root.cancellations.length + " cancelled"
              : String(root.departures.length)
            foreground: root.foreground
            fontFamily: root.fontFamily

            iconComponent: Component {
              Text {
                text: root.glyph
                color: root.stale || root.errorMessage !== "" ? root.urgent : root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.display
              }
            }
          }

          BorderSurface {
            visible: root.errorMessage !== ""
            width: parent.width
            implicitHeight: visible ? errorText.implicitHeight + Style.space(20) : 0
            color: Qt.rgba(root.urgent.r, root.urgent.g, root.urgent.b, 0.10)
            borderSpec: Border.flat(Qt.rgba(root.urgent.r, root.urgent.g, root.urgent.b, 0.35), 1)
            radius: Style.cornerRadius

            Text {
              id: errorText
              anchors.fill: parent
              anchors.margins: Style.space(10)
              textFormat: Text.PlainText
              text: root.errorDetail + (root.report ? "\nShowing the last successful result." : "")
              color: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              wrapMode: Text.WordWrap
            }
          }

          Text {
            visible: !root.loading && root.errorMessage === ""
              && root.departures.length === 0
            width: parent.width
            topPadding: Style.space(20)
            bottomPadding: Style.space(20)
            text: Model.emptyStateLabel(root.cancellations.length)
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.body
            horizontalAlignment: Text.AlignHCenter
          }

          Repeater {
            model: root.departures

            BorderSurface {
              required property var modelData
              width: panelColumn.width
              implicitHeight: rowLayout.implicitHeight + Style.space(20)
              color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.05)
              borderSpec: Border.controlSpec("normal", root.foreground, Color.accent)
              radius: Style.cornerRadius

              RowLayout {
                id: rowLayout
                anchors.fill: parent
                anchors.margins: Style.space(10)
                spacing: Style.space(12)

                Text {
                  text: modelData.line
                  color: root.foreground
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.title
                  font.bold: true
                  Layout.alignment: Qt.AlignVCenter
                  Layout.preferredWidth: Style.space(42)
                }

                ColumnLayout {
                  spacing: Style.space(2)
                  Layout.fillWidth: true
                  Layout.alignment: Qt.AlignVCenter

                  Text {
                    textFormat: Text.PlainText
                    text: modelData.headsign
                    color: root.foreground
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.body
                    font.bold: true
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                  }

                  Text {
                    textFormat: Text.PlainText
                    text: modelData.boardingPoint
                      + (modelData.platform ? " · " + modelData.platform : "")
                    color: root.dim
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.caption
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                  }
                }

                ColumnLayout {
                  spacing: Style.space(2)
                  Layout.alignment: Qt.AlignRight | Qt.AlignVCenter

                  Text {
                    text: Model.leaveLabel(modelData.leaveSeconds)
                    color: root.foreground
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.body
                    font.bold: true
                    horizontalAlignment: Text.AlignRight
                    Layout.alignment: Qt.AlignRight
                  }

                  Text {
                    text: Model.departureTimingLabel(
                      modelData.departsAtMs,
                      modelData.delaySeconds
                    )
                    color: root.dim
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.caption
                    horizontalAlignment: Text.AlignRight
                    Layout.alignment: Qt.AlignRight
                  }
                }
              }
            }
          }

          Repeater {
            model: root.cancellations

            BorderSurface {
              required property var modelData
              width: panelColumn.width
              implicitHeight: cancellationColumn.implicitHeight + Style.space(20)
              color: Qt.rgba(root.urgent.r, root.urgent.g, root.urgent.b, 0.10)
              borderSpec: Border.flat(Qt.rgba(root.urgent.r, root.urgent.g, root.urgent.b, 0.35), 1)
              radius: Style.cornerRadius

              Column {
                id: cancellationColumn
                anchors.fill: parent
                anchors.margins: Style.space(10)
                spacing: Style.space(2)

                Text {
                  width: parent.width
                  textFormat: Text.PlainText
                  text: "Cancelled · " + modelData.line + " → " + modelData.headsign
                  color: root.foreground
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.body
                  font.bold: true
                  elide: Text.ElideRight
                }

                Text {
                  width: parent.width
                  textFormat: Text.PlainText
                  text: modelData.boardingPoint + " · "
                    + Model.cancellationLabel(modelData.departsAtMs, modelData.departsSeconds)
                  color: root.dim
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  wrapMode: Text.WordWrap
                }
              }
            }
          }

          Text {
            visible: root.loading && !!root.report
            width: parent.width
            text: "Refreshing…"
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
            horizontalAlignment: Text.AlignHCenter
          }
        }
      }
    }
  }
}
