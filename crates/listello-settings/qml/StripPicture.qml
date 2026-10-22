import QtQuick
import QtQuick.Window
import org.kde.kirigami as Kirigami

// The strip as the Touch Bar will show it. Click a widget to pick it; drag it sideways to
// move it. Drawn at the panel's own pixels where the window is wide enough, so it is as sharp
// as the panel.
Item {
    id: picture

    required property var settings
    // Panel pixels to window points.
    readonly property real scale: bar.width / settings.panelWidth
    property int pressed: -1
    property bool dragging: false
    property real pressX: 0
    property real dragX: 0
    property int slot: -1

    implicitHeight: frame.height

    Rectangle {
        id: frame
        anchors.horizontalCenter: parent.horizontalCenter
        width: Math.min(parent.width,
                        settings.panelWidth / Screen.devicePixelRatio + Kirigami.Units.largeSpacing * 2)
        height: bar.height + Kirigami.Units.largeSpacing * 2
        radius: Kirigami.Units.cornerRadius * 2
        color: "#1b1b1b"

        Image {
            id: bar
            anchors.centerIn: parent
            width: parent.width - Kirigami.Units.largeSpacing * 2
            height: width * settings.panelHeight / settings.panelWidth
            source: settings.picture
            smooth: true
            mipmap: true
            cache: false
        }

        // The widget picked.
        Rectangle {
            visible: settings.selected >= 0 && !picture.dragging
                     && (settings.revision, settings.spanWidth(settings.selected)) > 0
            x: bar.x + (settings.revision, settings.spanX(settings.selected)) * picture.scale - 2
            y: bar.y - 3
            width: (settings.revision, settings.spanWidth(settings.selected)) * picture.scale + 4
            height: bar.height + 6
            radius: Kirigami.Units.cornerRadius
            color: "transparent"
            border.color: Kirigami.Theme.highlightColor
            border.width: 2
        }

        // The widget under the mouse.
        Rectangle {
            visible: hover.index >= 0 && hover.index !== settings.selected && !picture.dragging
            x: bar.x + (settings.revision, settings.spanX(hover.index)) * picture.scale - 2
            y: bar.y - 3
            width: (settings.revision, settings.spanWidth(hover.index)) * picture.scale + 4
            height: bar.height + 6
            radius: Kirigami.Units.cornerRadius
            color: Qt.rgba(1, 1, 1, 0.08)
            border.color: Qt.rgba(1, 1, 1, 0.25)
            border.width: 1
        }

        // While dragging: the widget lifted, and where it will land.
        Rectangle {
            visible: picture.dragging
            x: bar.x + settings.spanX(picture.pressed) * picture.scale + (picture.dragX - picture.pressX)
            y: bar.y - 3
            width: settings.spanWidth(picture.pressed) * picture.scale
            height: bar.height + 6
            radius: Kirigami.Units.cornerRadius
            color: Qt.rgba(Kirigami.Theme.highlightColor.r, Kirigami.Theme.highlightColor.g,
                           Kirigami.Theme.highlightColor.b, 0.35)
            border.color: Kirigami.Theme.highlightColor
            border.width: 2
        }
        Rectangle {
            visible: picture.dragging && picture.slot >= 0
            x: bar.x + settings.slotX(picture.slot) * picture.scale - width / 2
            y: bar.y - 6
            width: 4
            height: bar.height + 12
            radius: 2
            color: Kirigami.Theme.highlightColor
        }

        MouseArea {
            id: hover
            property int index: -1
            anchors.fill: bar
            hoverEnabled: true
            cursorShape: picture.dragging ? Qt.ClosedHandCursor
                       : index >= 0 ? Qt.PointingHandCursor : Qt.ArrowCursor
            onExited: index = -1
            onPositionChanged: (mouse) => {
                index = settings.widgetAt(mouse.x / picture.scale)
                if (picture.pressed >= 0) {
                    picture.dragX = mouse.x
                    if (!picture.dragging && Math.abs(mouse.x - picture.pressX) > Kirigami.Units.gridUnit / 2)
                        picture.dragging = true
                    if (picture.dragging)
                        picture.slot = settings.dropSlot(mouse.x / picture.scale)
                }
            }
            onPressed: (mouse) => {
                picture.pressed = settings.widgetAt(mouse.x / picture.scale)
                picture.pressX = mouse.x
                picture.dragX = mouse.x
                settings.selected = picture.pressed
            }
            onReleased: {
                if (picture.dragging && picture.slot >= 0)
                    settings.moveWidget(picture.pressed, picture.slot)
                picture.dragging = false
                picture.pressed = -1
                picture.slot = -1
            }
        }
    }
}
