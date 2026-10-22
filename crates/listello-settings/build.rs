use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("org.listello.settings").qml_files([
        "qml/Main.qml",
        "qml/StripPicture.qml",
        "qml/WidgetOptions.qml",
        "qml/GeneralOptions.qml",
        "qml/IconPicker.qml",
        "qml/KeyField.qml",
    ]))
    .qt_module("Widgets")
    .qt_module("Quick")
    .qt_module("QuickControls2")
    .files(["src/bridge.rs", "src/tray.rs"])
    .cpp_file("cpp/app.cpp")
    .cpp_file("cpp/tray.cpp")
    .build();
}
