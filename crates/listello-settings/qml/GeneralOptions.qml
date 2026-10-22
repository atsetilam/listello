import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// [general] and the strip-wide settings in [strip].
Kirigami.FormLayout {
    id: general

    required property var settings
    required property var t

    readonly property int rev: settings.revision

    function str(section, key) { return rev, settings.sectionStr(section, key) }
    function num(section, key, fallback) { return rev, settings.sectionNum(section, key, fallback) }

    QQC2.ComboBox {
        Kirigami.FormData.label: general.t("language")
        readonly property var values: ["system", "italian", "english"]
        model: [general.t("language_system"), "Italiano", "English"]
        currentIndex: Math.max(0, values.indexOf(general.str("general", "language") || "system"))
        // "system" is the default, so it is written as nothing at all.
        onActivated: (index) => settings.setSectionStr("general", "language",
                                                      index === 0 ? "" : values[index])
    }
    QQC2.Label {
        text: general.t("language_hint")
        color: Kirigami.Theme.disabledTextColor
        font: Kirigami.Theme.smallFont
        // Explanations wrap at a readable width instead of running past the window's edge.
        wrapMode: Text.WordWrap
        Layout.maximumWidth: Kirigami.Units.gridUnit * 30
    }

    Kirigami.Separator { Kirigami.FormData.isSection: true }

    QQC2.ComboBox {
        id: previews
        Kirigami.FormData.label: general.t("previews")
        readonly property var values: ["off", "snapshots", "live"]
        readonly property string current: general.str("strip", "window_previews") || "off"
        model: [general.t("previews_off"), general.t("previews_snapshots"), general.t("previews_live")]
        currentIndex: Math.max(0, values.indexOf(current))
        onActivated: (index) => settings.setSectionStr("strip", "window_previews",
                                                      index === 0 ? "" : values[index])
    }
    QQC2.ComboBox {
        Kirigami.FormData.label: general.t("preview_style")
        enabled: previews.current !== "off"
        readonly property var values: ["fill", "whole", "stretch"]
        model: [general.t("style_fill"), general.t("style_whole"), general.t("style_stretch")]
        currentIndex: Math.max(0, values.indexOf(general.str("strip", "preview_style") || "fill"))
        onActivated: (index) => settings.setSectionStr("strip", "preview_style",
                                                      index === 0 ? "" : values[index])
    }

    Kirigami.Separator { Kirigami.FormData.isSection: true }

    RowLayout {
        Kirigami.FormData.label: general.t("hold")
        QQC2.SpinBox {
            from: 1
            to: 50
            editable: true
            value: Math.round(general.num("strip", "hold_seconds", 0.5) * 10)
            textFromValue: (v) => (v / 10).toLocaleString(Qt.locale(), 'f', 1)
            valueFromText: (text) => Math.round(Number.fromLocaleString(Qt.locale(), text) * 10)
            onValueModified: settings.setSectionNum("strip", "hold_seconds", value / 10)
        }
        QQC2.Label { text: general.t("seconds") }
    }
    RowLayout {
        Kirigami.FormData.label: general.t("controls")
        QQC2.SpinBox {
            from: 1
            to: 60
            editable: true
            value: Math.round(general.num("strip", "controls_seconds", 5))
            onValueModified: settings.setSectionNum("strip", "controls_seconds", value)
        }
        QQC2.Label { text: general.t("seconds") }
    }
    RowLayout {
        Kirigami.FormData.label: general.t("gap")
        QQC2.SpinBox {
            from: 0
            to: 100
            editable: true
            value: Math.round(general.num("strip", "gap", 8))
            onValueModified: settings.setSectionNum("strip", "gap", value)
        }
        QQC2.Label { text: general.t("pixels") }
    }

    Kirigami.Separator { Kirigami.FormData.isSection: true }

    // Acts at once, like Reload and Stop: it is not in the file, it is systemd's.
    QQC2.CheckBox {
        id: atLogin
        Kirigami.FormData.label: general.t("login")
        text: general.t("login_start")
        checked: settings.startAtLogin()
        onToggled: settings.setStartAtLogin(checked)
        // Asked again when the window comes back to the front, since it can change elsewhere.
        Connections {
            target: Window.window
            function onActiveChanged() {
                if (Window.window.active) atLogin.checked = settings.startAtLogin()
            }
        }
    }
    QQC2.Label {
        text: general.t("login_hint")
        color: Kirigami.Theme.disabledTextColor
        font: Kirigami.Theme.smallFont
        // Explanations wrap at a readable width instead of running past the window's edge.
        wrapMode: Text.WordWrap
        Layout.maximumWidth: Kirigami.Units.gridUnit * 30
    }

    QQC2.CheckBox {
        Kirigami.FormData.label: general.t("tray")
        text: general.t("tray_show")
        checked: (general.rev, settings.sectionBool("general", "tray"))
        onToggled: settings.setSectionBool("general", "tray", checked)
    }
    // What the tray offers is here too (D8 amendment). It acts at once: it changes nothing in
    // the file.
    QQC2.Button {
        Kirigami.FormData.label: "Touch Bar:"
        text: general.t("reload")
        icon.name: "view-refresh"
        onClicked: settings.reloadListello()
    }
    QQC2.Label {
        text: general.t("reload_hint")
        color: Kirigami.Theme.disabledTextColor
        font: Kirigami.Theme.smallFont
        // Explanations wrap at a readable width instead of running past the window's edge.
        wrapMode: Text.WordWrap
        Layout.maximumWidth: Kirigami.Units.gridUnit * 30
    }
    QQC2.Button {
        text: general.t("stop")
        icon.name: "media-playback-stop"
        // Everything of listello's stops, this window included.
        onClicked: {
            settings.stopListello()
            Qt.quit()
        }
    }
    QQC2.Label {
        text: general.t("stop_hint")
        color: Kirigami.Theme.disabledTextColor
        font: Kirigami.Theme.smallFont
        // Explanations wrap at a readable width instead of running past the window's edge.
        wrapMode: Text.WordWrap
        Layout.maximumWidth: Kirigami.Units.gridUnit * 30
    }
}
