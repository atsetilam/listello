import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// One of listello's packaged icons: the icon as the Touch Bar draws it, and a list to pick
// another from.
RowLayout {
    id: picker

    required property var settings
    // The name in the file, "" for none.
    property string current: ""
    // "folder" for a folder's icons, "" for a button's.
    property string context: ""
    // Whether "None" is offered.
    property bool optional: false
    property string noneText: ""
    signal picked(string name)

    readonly property var names: {
        var out = optional ? [""] : []
        for (var i = 0; i < settings.iconCount(context); i++)
            out.push(settings.iconName(context, i))
        if (current !== "" && out.indexOf(current) < 0)
            out.push(current)
        return out
    }

    spacing: Kirigami.Units.smallSpacing

    Swatch {
        name: picker.current
        source: picker.settings.iconUrl(picker.current)
        visible: picker.current !== ""
    }

    QQC2.ComboBox {
        id: combo
        Layout.preferredWidth: Kirigami.Units.gridUnit * 11
        model: picker.names.map((n) => n === "" ? picker.noneText : n)
        currentIndex: picker.names.indexOf(picker.current)
        onActivated: (index) => picker.picked(picker.names[index])
        delegate: QQC2.ItemDelegate {
            required property int index
            required property var modelData
            width: ListView.view ? ListView.view.width : implicitWidth
            highlighted: combo.highlightedIndex === index
            contentItem: RowLayout {
                spacing: Kirigami.Units.smallSpacing
                Swatch { name: picker.names[index]; source: picker.settings.iconUrl(picker.names[index]) }
                QQC2.Label {
                    Layout.fillWidth: true
                    text: modelData
                }
            }
        }
    }

    // An icon on the Touch Bar's black.
    component Swatch: Rectangle {
        property string name
        property alias source: image.source
        implicitWidth: Kirigami.Units.iconSizes.smallMedium + 6
        implicitHeight: implicitWidth
        radius: 3
        color: "#000000"
        opacity: name === "" ? 0 : 1
        Image {
            id: image
            anchors.centerIn: parent
            width: Kirigami.Units.iconSizes.smallMedium
            height: width
            sourceSize.width: width * 2
            sourceSize.height: height * 2
        }
    }
}
