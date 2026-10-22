#pragma once
#include <cstdint>

// Starts the window and returns its exit code. A QApplication rather than a QGuiApplication:
// KDE's desktop style for Qt Quick Controls draws through QStyle, which needs one.
::std::int32_t run_settings_window();
