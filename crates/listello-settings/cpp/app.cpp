#include "app.h"
#include "listello-settings/src/tray.cxxqt.h"

#include <QApplication>
#include <QIcon>
#include <QMessageBox>
#include <QPushButton>
#include <QQmlApplicationEngine>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QTimer>

static QString text(const char *key)
{
    const rust::String s = tray_text(key);
    return QString::fromUtf8(s.data(), static_cast<qsizetype>(s.size()));
}

// Opening the window is how a user starts listello (D8, second amendment): the very first time
// ever it asks whether to start it at every login, then it starts it if it is stopped. While the
// window is open it follows the agent: once listello is stopped, from here or from the tray,
// the window closes.
static void start_listello_for_the_window(QApplication &app)
{
    if (!lifecycle_enabled()) {
        return;
    }
    if (first_start_needed()) {
        QMessageBox box(QMessageBox::Question, QStringLiteral("listello"), text("first_question"));
        QPushButton *every = box.addButton(text("first_yes"), QMessageBox::AcceptRole);
        QPushButton *now = box.addButton(text("first_no"), QMessageBox::RejectRole);
        box.setDefaultButton(every);
        box.setEscapeButton(now);
        box.exec();
        answer_first_start(box.clickedButton() == every);
    }
    ensure_started();
    auto *watch = new QTimer(&app);
    QObject::connect(watch, &QTimer::timeout, &app, []() {
        if (listello_stopped()) {
            QCoreApplication::quit();
        }
    });
    watch->start(1500);
}

::std::int32_t run_settings_window()
{
    static int argc = 1;
    static char name[] = "listello-settings";
    static char *argv[] = {name, nullptr};
    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("listello-settings"));
    QApplication::setDesktopFileName(QStringLiteral("org.listello.Settings"));
    QApplication::setWindowIcon(QIcon::fromTheme(QStringLiteral("listello")));
    if (qEnvironmentVariableIsEmpty("QT_QUICK_CONTROLS_STYLE")) {
        QQuickStyle::setStyle(QStringLiteral("org.kde.desktop"));
    }
    start_listello_for_the_window(app);
    QQmlApplicationEngine engine;
    engine.load(QUrl(QStringLiteral("qrc:/qt/qml/org/listello/settings/qml/Main.qml")));
    if (engine.rootObjects().isEmpty()) {
        return 1;
    }
    // For looking at the window without a screen: LISTELLO_SETTINGS_SCREENSHOT=out.png draws
    // it once, saves it and quits.
    const QString shot = qEnvironmentVariable("LISTELLO_SETTINGS_SCREENSHOT");
    if (!shot.isEmpty()) {
        auto *window = qobject_cast<QQuickWindow *>(engine.rootObjects().first());
        QTimer::singleShot(1500, &app, [window, shot]() {
            if (window) {
                window->grabWindow().save(shot);
            }
            QCoreApplication::quit();
        });
    }
    return app.exec();
}
