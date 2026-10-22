#pragma once
#include <cstdint>

// Runs the system-tray icon and returns its exit code; 0 at once when the tray is off or
// already running.
::std::int32_t run_tray();
