import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import org.listello.settings

Kirigami.ApplicationWindow {
    id: root

    // Every text goes through here. Mentioning `uiLang` makes each binding re-read when the
    // language changes.
    function t(key) {
        return bridge.uiLang, bridge.tr(key)
    }

    title: t("window_title")
    width: Kirigami.Units.gridUnit * 64
    height: Kirigami.Units.gridUnit * 40
    minimumWidth: Kirigami.Units.gridUnit * 40
    minimumHeight: Kirigami.Units.gridUnit * 26

    Settings {
        id: bridge
        Component.onCompleted: {
            refresh()
            // LISTELLO_SETTINGS_SHOW="tab,widget" opens on that tab with that widget picked,
            // for looking at one panel without clicking (see the screenshot hook in app.cpp).
            const show = startView()
            if (show.length === 2) {
                tabs.currentIndex = show[0]
                bridge.selected = show[1]
            }
        }
    }

    pageStack.globalToolBar.style: Kirigami.ApplicationHeaderStyle.None
    pageStack.initialPage: Kirigami.Page {
        id: page
        padding: 0
        title: root.t("window_title")

        header: QQC2.TabBar {
            id: tabs
            QQC2.TabButton { text: root.t("strip"); width: tabs.width / 3 }
            QQC2.TabButton { text: root.t("fn_keys"); width: tabs.width / 3 }
            QQC2.TabButton { text: root.t("general"); width: tabs.width / 3 }
            onCurrentIndexChanged: if (currentIndex < 2) bridge.setStripLayer(currentIndex)
        }

        footer: QQC2.ToolBar {
            position: QQC2.ToolBar.Footer
            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing
                QQC2.Button {
                    text: root.t("restore")
                    icon.name: "edit-reset"
                    enabled: bridge.ownLayout
                    onClicked: restoreDialog.open()
                }
                Item { Layout.fillWidth: true }
                QQC2.Button {
                    text: root.t("cancel")
                    icon.name: "dialog-cancel"
                    enabled: bridge.dirty
                    onClicked: bridge.cancel()
                }
                QQC2.Button {
                    text: root.t("apply")
                    icon.name: "dialog-ok-apply"
                    enabled: (bridge.dirty || !bridge.ownLayout) && bridge.error === ""
                    onClicked: {
                        // Turning the tray on or off alone sends nothing to the Touch Bar, so
                        // it says nothing.
                        const reaches = bridge.changesTouchBar()
                        if (bridge.apply() && reaches) applied.visible = true
                    }
                }
            }
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            Kirigami.InlineMessage {
                id: applied
                Layout.fillWidth: true
                Layout.margins: Kirigami.Units.smallSpacing
                type: Kirigami.MessageType.Positive
                text: root.t("applied")
                showCloseButton: true
                visible: false
                Connections {
                    target: bridge
                    function onDirtyChanged() { if (bridge.dirty) applied.visible = false }
                }
            }
            Kirigami.InlineMessage {
                Layout.fillWidth: true
                Layout.margins: Kirigami.Units.smallSpacing
                type: Kirigami.MessageType.Information
                text: root.t("machine_layout")
                visible: !bridge.ownLayout && !bridge.dirty
            }
            Kirigami.InlineMessage {
                Layout.fillWidth: true
                Layout.margins: Kirigami.Units.smallSpacing
                type: Kirigami.MessageType.Error
                text: root.t("invalid") + " " + bridge.error
                visible: bridge.error !== ""
            }

            // The strip and the Fn keys: the picture, the row of widgets to add, and the
            // options of the one picked.
            ColumnLayout {
                visible: tabs.currentIndex < 2
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: Kirigami.Units.largeSpacing

                StripPicture {
                    Layout.fillWidth: true
                    Layout.topMargin: Kirigami.Units.gridUnit * 1.5
                    Layout.leftMargin: Kirigami.Units.largeSpacing * 2
                    Layout.rightMargin: Kirigami.Units.largeSpacing * 2
                    settings: bridge
                }

                QQC2.Label {
                    Layout.alignment: Qt.AlignHCenter
                    // With nothing picked, the area below already says how.
                    opacity: bridge.selected >= 0 || bridge.widgetCount === 0 ? 1 : 0
                    text: bridge.layer === 1 && bridge.widgetCount === 0
                        ? root.t("fn_empty") : root.t("pick_hint")
                    color: Kirigami.Theme.disabledTextColor
                }

                Flow {
                    Layout.fillWidth: true
                    Layout.leftMargin: Kirigami.Units.largeSpacing * 2
                    Layout.rightMargin: Kirigami.Units.largeSpacing * 2
                    spacing: Kirigami.Units.smallSpacing
                    QQC2.Label {
                        text: root.t("add")
                        height: Kirigami.Units.gridUnit * 2
                        verticalAlignment: Text.AlignVCenter
                        rightPadding: Kirigami.Units.smallSpacing
                    }
                    Repeater {
                        model: bridge.kindCount()
                        QQC2.Button {
                            required property int index
                            text: root.t("kind_" + bridge.kindAt(index))
                            icon.name: "list-add"
                            flat: true
                            onClicked: bridge.addWidget(bridge.kindAt(index))
                        }
                    }
                }

                Kirigami.Separator { Layout.fillWidth: true }

                QQC2.ScrollView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    contentWidth: availableWidth
                    WidgetOptions {
                        width: parent.width
                        settings: bridge
                        t: root.t
                    }
                }
            }

            QQC2.ScrollView {
                visible: tabs.currentIndex === 2
                Layout.fillWidth: true
                Layout.fillHeight: true
                contentWidth: availableWidth
                // Room between the tabs and the first setting.
                ColumnLayout {
                    width: parent.width
                    GeneralOptions {
                        Layout.fillWidth: true
                        Layout.topMargin: Kirigami.Units.gridUnit * 1.5
                        settings: bridge
                        t: root.t
                    }
                }
            }
        }
    }

    Kirigami.PromptDialog {
        id: restoreDialog
        title: root.t("restore")
        subtitle: root.t("restore_question")
        standardButtons: Kirigami.Dialog.NoButton
        customFooterActions: [
            Kirigami.Action {
                text: root.t("restore_confirm")
                icon.name: "edit-reset"
                onTriggered: {
                    bridge.restoreDefault()
                    restoreDialog.close()
                }
            },
            Kirigami.Action {
                text: root.t("cancel")
                icon.name: "dialog-cancel"
                onTriggered: restoreDialog.close()
            }
        ]
    }
}
