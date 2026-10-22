import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// The keys a button presses: click, then press them (a combination too, Ctrl + C), or pick
// one of the keys a window never receives (volume, brightness, F13…).
RowLayout {
    id: field

    required property var settings
    required property var t
    // As in the file: "LeftCtrl, C".
    property string keys: ""
    signal edited(string keys)

    property bool recording: false
    property var pressed: []
    property int held: 0

    spacing: Kirigami.Units.smallSpacing

    QQC2.Button {
        id: record
        Layout.preferredWidth: Kirigami.Units.gridUnit * 10
        icon.name: field.recording ? "media-record" : "input-keyboard"
        text: field.recording ? field.t("press_keys")
            : field.keys === "" ? field.t("no_keys")
            : field.settings.keyLabel(field.keys)
        highlighted: field.recording
        onClicked: {
            field.pressed = []
            field.held = 0
            field.recording = true
            catcher.forceActiveFocus()
        }
        QQC2.ToolTip.text: field.t("record_hint")
        QQC2.ToolTip.visible: hovered && !field.recording
    }

    // Takes the keys while recording. Every press is kept, in order; when the last key is
    // let go, that is the combination.
    Item {
        id: catcher
        width: 0
        height: 0
        Keys.onPressed: (event) => {
            if (!field.recording)
                return
            event.accepted = true
            if (event.isAutoRepeat)
                return
            const name = field.settings.keyFromScanCode(event.nativeScanCode)
            if (name !== "" && field.pressed.indexOf(name) < 0)
                field.pressed = field.pressed.concat([name])
            field.held += 1
        }
        Keys.onReleased: (event) => {
            if (!field.recording)
                return
            event.accepted = true
            if (event.isAutoRepeat)
                return
            field.held -= 1
            if (field.held <= 0 && field.pressed.length > 0) {
                field.recording = false
                field.edited(field.pressed.join(", "))
            }
        }
        onActiveFocusChanged: if (!activeFocus) field.recording = false
    }

    QQC2.ComboBox {
        Layout.preferredWidth: Kirigami.Units.gridUnit * 10
        displayText: field.t("special_key")
        model: {
            var out = []
            for (var i = 0; i < field.settings.specialKeyCount(); i++)
                out.push(field.settings.keyLabel(field.settings.specialKey(i)))
            return out
        }
        onActivated: (index) => field.edited(field.settings.specialKey(index))
    }
}
