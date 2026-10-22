#include "tray.h"
#include "listello-settings/src/tray.cxxqt.h"

#include <QAction>
#include <QApplication>
#include <QFileSystemWatcher>
#include <QIcon>
#include <QMenu>
#include <QSystemTrayIcon>

static QString text(const char *key)
{
    const rust::String s = tray_text(key);
    return QString::fromUtf8(s.data(), static_cast<qsizetype>(s.size()));
}

::std::int32_t run_tray()
{
    if (!tray_enabled() || !take_tray_lock()) {
        return 0;
    }
    static int argc = 1;
    static char name[] = "listello-settings";
    static char *argv[] = {name, nullptr};
    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("listello-tray"));
    QApplication::setDesktopFileName(QStringLiteral("org.listello.Settings"));
    QApplication::setQuitOnLastWindowClosed(false);

    QSystemTrayIcon icon(QIcon::fromTheme(QStringLiteral("listello")));
    icon.setToolTip(QStringLiteral("listello"));
    QMenu menu;
    QAction *open = menu.addAction(QIcon::fromTheme(QStringLiteral("configure")), QString());
    QAction *reload = menu.addAction(QIcon::fromTheme(QStringLiteral("view-refresh")), QString());
    QAction *stop = menu.addAction(QIcon::fromTheme(QStringLiteral("media-playback-stop")), QString());
    // There is no Start item: stopping listello takes the tray with it, and the application menu
    // starts it again.
    const auto translate = [open, reload, stop]() {
        open->setText(text("tray_open"));
        reload->setText(text("reload"));
        stop->setText(text("stop"));
    };
    translate();
    QObject::connect(&menu, &QMenu::aboutToShow, translate);
    QObject::connect(open, &QAction::triggered, []() { open_settings(); });
    QObject::connect(reload, &QAction::triggered, []() { reload_listello(); });
    QObject::connect(stop, &QAction::triggered, []() { stop_listello(); });
    // A left click opens the window; the right click shows the menu.
    QObject::connect(&icon, &QSystemTrayIcon::activated, [](QSystemTrayIcon::ActivationReason why) {
        if (why == QSystemTrayIcon::Trigger) {
            open_settings();
        }
    });
    icon.setContextMenu(&menu);
    icon.show();

    // The settings window replaces the file rather than writing it, so watch the directory.
    QFileSystemWatcher watcher;
    const rust::String dir = config_dir_text();
    watcher.addPath(QString::fromUtf8(dir.data(), static_cast<qsizetype>(dir.size())));
    QObject::connect(&watcher, &QFileSystemWatcher::directoryChanged, [&app, translate]() {
        if (!tray_enabled()) {
            app.quit();
        } else {
            translate();
        }
    });
    return app.exec();
}
