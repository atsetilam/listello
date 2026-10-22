import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// The options of the widget picked in the picture.
ColumnLayout {
    id: options

    required property var settings
    required property var t

    // Every read mentions this, so all of them re-read after any change or a new pick.
    readonly property int rev: settings.revision + settings.selected * 100000
    readonly property string kind: (rev, settings.selected >= 0 ? settings.kind(settings.selected) : "")
    readonly property real fieldWidth: Kirigami.Units.gridUnit * 16

    function str(key) { return rev, settings.getStr(key) }
    function bool(key, fallback) { return rev, settings.getBool(key, fallback) }

    spacing: Kirigami.Units.largeSpacing

    Kirigami.PlaceholderMessage {
        visible: options.kind === ""
        Layout.fillWidth: true
        Layout.topMargin: Kirigami.Units.gridUnit * 2
        icon.name: "input-touchscreen"
        text: options.t("pick_hint")
    }

    // The heading: what it is, and the buttons that act on it as a whole.
    RowLayout {
        visible: options.kind !== ""
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing * 2
        Layout.rightMargin: Kirigami.Units.largeSpacing * 2
        Layout.topMargin: Kirigami.Units.largeSpacing
        Kirigami.Heading {
            Layout.fillWidth: true
            level: 2
            text: options.t("kind_" + options.kind)
        }
        QQC2.ToolButton {
            icon.name: "go-previous"
            text: options.t("move_left")
            display: QQC2.AbstractButton.IconOnly
            QQC2.ToolTip.text: text
            QQC2.ToolTip.visible: hovered
            enabled: settings.selected > 0
            onClicked: settings.moveWidget(settings.selected, settings.selected - 1)
        }
        QQC2.ToolButton {
            icon.name: "go-next"
            text: options.t("move_right")
            display: QQC2.AbstractButton.IconOnly
            QQC2.ToolTip.text: text
            QQC2.ToolTip.visible: hovered
            enabled: settings.selected >= 0 && settings.selected < settings.widgetCount - 1
            onClicked: settings.moveWidget(settings.selected, settings.selected + 2)
        }
        QQC2.Button {
            icon.name: "edit-delete-remove"
            text: options.t("remove")
            onClicked: settings.removeSelected()
        }
    }

    Kirigami.FormLayout {
        visible: options.kind !== ""
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing * 2
        Layout.rightMargin: Kirigami.Units.largeSpacing * 2

        // --- button ---
        QQC2.TextField {
            visible: options.kind === "button"
            Kirigami.FormData.label: options.t("label")
            Layout.preferredWidth: options.fieldWidth
            text: options.str("label")
            onEditingFinished: settings.setStr("label", text)
        }
        IconPicker {
            visible: options.kind === "button"
            Kirigami.FormData.label: options.t("icon")
            settings: options.settings
            optional: true
            noneText: options.t("no_icon")
            current: options.str("icon")
            onPicked: (name) => settings.setStr("icon", name)
        }
        KeyField {
            visible: options.kind === "button"
            Kirigami.FormData.label: options.t("keys")
            settings: options.settings
            t: options.t
            keys: (options.rev, settings.getList("keys"))
            onEdited: (keys) => settings.setList("keys", keys)
        }

        // --- clock ---
        QQC2.ComboBox {
            id: clockFormat
            visible: options.kind === "clock"
            Kirigami.FormData.label: options.t("format")
            Layout.preferredWidth: options.fieldWidth
            readonly property var formats: ["%H:%M", "%a %-d %B %H:%M", "%a %-d %b %H:%M",
                                             "%d/%m/%y %H:%M", "%-d %B %Y %H:%M"]
            readonly property string current: options.str("format") || "%H:%M"
            readonly property var values: formats.concat(formats.indexOf(current) < 0 ? [current] : [])
            // Shown as today's date in that format, not as the format's codes.
            model: (options.rev, settings.uiLang, values.map((f) => settings.formatExample(f) || f))
            currentIndex: values.indexOf(current)
            onActivated: (index) => settings.setStr("format", values[index])
        }
        QQC2.TextField {
            visible: options.kind === "clock"
            Kirigami.FormData.label: options.t("format_custom")
            Layout.preferredWidth: options.fieldWidth
            font.family: "monospace"
            text: options.str("format") || "%H:%M"
            onEditingFinished: settings.setStr("format", text)
        }
        QQC2.ComboBox {
            visible: options.kind === "clock"
            Kirigami.FormData.label: options.t("align")
            readonly property var values: ["start", "center", "end"]
            model: [options.t("align_start"), options.t("align_center"), options.t("align_end")]
            currentIndex: Math.max(0, values.indexOf(options.str("align") || "center"))
            onActivated: (index) => settings.setStr("align", values[index])
        }

        // --- dock ---
        ColumnLayout {
            visible: options.kind === "dock"
            Kirigami.FormData.label: options.t("pinned")
            Kirigami.FormData.buddyFor: pinList
            spacing: Kirigami.Units.smallSpacing
            ColumnLayout {
                id: pinList
                spacing: 0
                Repeater {
                    model: (options.rev, settings.pinCount())
                    RowLayout {
                        required property int index
                        spacing: Kirigami.Units.smallSpacing
                        Image {
                            Layout.preferredWidth: Kirigami.Units.iconSizes.medium
                            Layout.preferredHeight: Kirigami.Units.iconSizes.medium
                            source: (options.rev, settings.pinIcon(index))
                            smooth: true
                        }
                        QQC2.Label {
                            Layout.preferredWidth: Kirigami.Units.gridUnit * 12
                            text: (options.rev, settings.pinName(index))
                            elide: Text.ElideRight
                        }
                        QQC2.ToolButton {
                            icon.name: "go-up"
                            enabled: index > 0
                            onClicked: settings.movePin(index, index - 1)
                            QQC2.ToolTip.text: options.t("move_up")
                            QQC2.ToolTip.visible: hovered
                        }
                        QQC2.ToolButton {
                            icon.name: "go-down"
                            enabled: index < settings.pinCount() - 1
                            onClicked: settings.movePin(index, index + 1)
                            QQC2.ToolTip.text: options.t("move_down")
                            QQC2.ToolTip.visible: hovered
                        }
                        QQC2.ToolButton {
                            icon.name: "edit-delete-remove"
                            onClicked: settings.removePin(index)
                            QQC2.ToolTip.text: options.t("remove")
                            QQC2.ToolTip.visible: hovered
                        }
                    }
                }
            }
            QQC2.ComboBox {
                Layout.preferredWidth: options.fieldWidth
                displayText: options.t("add_app")
                model: {
                    var out = []
                    for (var i = 0; i < settings.appCount(); i++)
                        out.push(settings.appName(i))
                    return out
                }
                onActivated: (index) => settings.addPin(index)
            }
            QQC2.Label {
                text: options.t("pinned_hint")
                color: Kirigami.Theme.disabledTextColor
                font: Kirigami.Theme.smallFont
            }
        }

        // --- battery, network ---
        QQC2.CheckBox {
            visible: options.kind === "battery"
            text: options.t("show_percentage")
            checked: options.bool("show_percentage", true)
            onToggled: settings.setBool("show_percentage", checked)
        }
        QQC2.CheckBox {
            visible: options.kind === "battery"
            text: options.t("show_bar")
            checked: options.bool("show_bar", true)
            onToggled: settings.setBool("show_bar", checked)
        }
        QQC2.CheckBox {
            visible: options.kind === "network"
            text: options.t("show_ssid")
            checked: options.bool("show_ssid", false)
            onToggled: settings.setBool("show_ssid", checked)
        }

        // --- control centre ---
        ColumnLayout {
            visible: options.kind === "control_center"
            Kirigami.FormData.label: options.t("buttons")
            spacing: Kirigami.Units.smallSpacing
            Repeater {
                model: (options.rev, options.kind === "control_center" ? settings.itemCount("button") : 0)
                RowLayout {
                    id: ccRow
                    required property int index
                    readonly property string system: (options.rev, settings.itemStr("button", index, "system"))
                    spacing: Kirigami.Units.smallSpacing
                    IconPicker {
                        settings: options.settings
                        current: (options.rev, settings.itemStr("button", ccRow.index, "icon"))
                        onPicked: (name) => settings.setItemStr("button", ccRow.index, "icon", name)
                    }
                    QQC2.TextField {
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                        placeholderText: options.t("label")
                        text: (options.rev, settings.itemStr("button", ccRow.index, "label"))
                        onEditingFinished: settings.setItemStr("button", ccRow.index, "label", text)
                    }
                    QQC2.ComboBox {
                        model: [options.t("action_keys"), options.t("action_lock")]
                        currentIndex: ccRow.system === "lock_session" ? 1 : 0
                        onActivated: (i) => {
                            if (i === 1)
                                settings.setItemStr("button", ccRow.index, "system", "lock_session")
                            else
                                settings.setItemStr("button", ccRow.index, "keys", "Mute")
                        }
                    }
                    KeyField {
                        visible: ccRow.system !== "lock_session"
                        settings: options.settings
                        t: options.t
                        keys: (options.rev, settings.itemStr("button", ccRow.index, "keys"))
                        onEdited: (keys) => settings.setItemStr("button", ccRow.index, "keys", keys)
                    }
                    QQC2.ToolButton {
                        icon.name: "go-up"
                        enabled: ccRow.index > 0
                        onClicked: settings.moveItem("button", ccRow.index, ccRow.index - 1)
                    }
                    QQC2.ToolButton {
                        icon.name: "go-down"
                        enabled: ccRow.index < settings.itemCount("button") - 1
                        onClicked: settings.moveItem("button", ccRow.index, ccRow.index + 1)
                    }
                    QQC2.ToolButton {
                        icon.name: "edit-delete-remove"
                        onClicked: settings.removeItem("button", ccRow.index)
                    }
                }
            }
            QQC2.Button {
                icon.name: "list-add"
                text: options.t("add_button")
                onClicked: settings.addItem("button")
            }
        }

        // --- folders ---
        ColumnLayout {
            visible: options.kind === "folders"
            Kirigami.FormData.label: options.t("folders")
            spacing: Kirigami.Units.largeSpacing
            Repeater {
                model: (options.rev, options.kind === "folders" ? settings.itemCount("folder") : 0)
                RowLayout {
                    id: folderRow
                    required property int index
                    spacing: Kirigami.Units.smallSpacing
                    IconPicker {
                        settings: options.settings
                        context: "folder"
                        current: (options.rev, settings.itemStr("folder", folderRow.index, "icon"))
                        onPicked: (name) => settings.setItemStr("folder", folderRow.index, "icon", name)
                    }
                    QQC2.TextField {
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 6
                        placeholderText: options.t("label")
                        text: (options.rev, settings.itemStr("folder", folderRow.index, "label"))
                        onEditingFinished: settings.setItemStr("folder", folderRow.index, "label", text)
                    }
                    QQC2.TextField {
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 10
                        placeholderText: options.t("path")
                        text: (options.rev, settings.itemStr("folder", folderRow.index, "path"))
                        onEditingFinished: settings.setItemStr("folder", folderRow.index, "path", text)
                    }
                    IconPicker {
                        settings: options.settings
                        context: "folder"
                        optional: true
                        noneText: options.t("same_when_empty")
                        current: (options.rev, settings.itemStr("folder", folderRow.index, "empty_icon"))
                        onPicked: (name) => settings.setItemStr("folder", folderRow.index, "empty_icon", name)
                    }
                    QQC2.ToolButton {
                        icon.name: "go-up"
                        enabled: folderRow.index > 0
                        onClicked: settings.moveItem("folder", folderRow.index, folderRow.index - 1)
                    }
                    QQC2.ToolButton {
                        icon.name: "go-down"
                        enabled: folderRow.index < settings.itemCount("folder") - 1
                        onClicked: settings.moveItem("folder", folderRow.index, folderRow.index + 1)
                    }
                    QQC2.ToolButton {
                        icon.name: "edit-delete-remove"
                        onClicked: settings.removeItem("folder", folderRow.index)
                    }
                }
            }
            QQC2.Button {
                icon.name: "list-add"
                text: options.t("add_folder")
                onClicked: settings.addItem("folder")
            }
        }

        // --- width, for all of them ---
        Kirigami.Separator {
            Kirigami.FormData.isSection: true
            visible: options.kind !== ""
        }
        RowLayout {
            visible: options.kind !== "" && options.kind !== "separator"
            Kirigami.FormData.label: options.t("width")
            readonly property string mode: (options.rev, settings.sizingMode())
            QQC2.ComboBox {
                id: widthMode
                readonly property var values: ["auto", "fixed", "share"]
                model: [options.t("width_auto"), options.t("width_fixed"), options.t("width_share")]
                currentIndex: Math.max(0, values.indexOf(parent.mode))
                onActivated: (index) => settings.setSizing(values[index],
                                                           values[index] === "fixed" ? 120 : 1)
            }
            QQC2.SpinBox {
                visible: parent.mode === "fixed"
                from: 1
                to: 4000
                stepSize: 4
                editable: true
                value: (options.rev, Math.round(settings.sizingAmount()))
                onValueModified: settings.setSizing("fixed", value)
            }
            QQC2.SpinBox {
                visible: parent.mode === "share"
                from: 1
                to: 100
                editable: true
                value: (options.rev, Math.round(settings.sizingAmount() * 10))
                textFromValue: (v) => (v / 10).toLocaleString(Qt.locale(), 'f', 1)
                valueFromText: (text) => Math.round(Number.fromLocaleString(Qt.locale(), text) * 10)
                onValueModified: settings.setSizing("share", value / 10)
            }
        }
        QQC2.SpinBox {
            visible: options.kind === "separator"
            Kirigami.FormData.label: options.t("width")
            from: 1
            to: 200
            editable: true
            value: (options.rev, Math.round(settings.sizingAmount()) || 2)
            onValueModified: settings.setSizing("fixed", value)
        }
    }

    Item { Layout.fillHeight: true }
}
