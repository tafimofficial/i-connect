# I Connect [Turbo Link]

High-Performance Wi-Fi & Hotspot Phone-to-PC Remote Controller & Bidirectional Batch File/Folder Transfer Engine — **Built from Scratch in Rust** with an **AMOLED Black + Electric Lime** theme.

---

## Pre-Built Release Binaries (`bin/`)
| Binary | Description |
|---|---|
| `bin/I_Connect_Installer.exe` | Native Rust Windows Setup Wizard (embeds server + icons + uninstaller + autostart + shortcuts + firewall rules) |
| `bin/I_Connect_Server.exe` | Standalone Rust Windows PC Server (AMOLED Black + Lime UI, System Tray, Drag & Drop, Hardware Input Injection) |
| `bin/I_Connect.apk` | Signed Universal Android APK (`arm64-v8a`, `armeabi-v7a`, `x86_64`) powered by Rust JNI (`libiconnect_core.so`) |

---

## Key Features
1. **Universal Keyboard & Mouse Simulation (`SendInput`)**: Hardware scan codes + Win32 Virtual Keys + Unicode fallback, smooth cursor acceleration, 1-finger tap, 2-finger tap, 2-finger scroll, double-tap & hold drag.
2. **5 Controller Tabs on Android**: Touchpad, Keyboard (live typing + modifiers + F1-F12 + arrows + volume), Media (Play/Pause, Prev, Next, -5s/+5s Seek, 0-100% Seekbar, Volume/Mute), System (Lock, Desktop, Close Window, Task Manager, Sleep, Restart, Shutdown), and Turbo Files.
3. **Persistent Background Service**: Android `ForegroundService` (`START_STICKY`) with `PARTIAL_WAKE_LOCK`, `WIFI_MODE_FULL_HIGH_PERF`, live notification speed & Disconnect button, surviving recent-apps swipes and screen lock.
4. **High-Speed Bidirectional Batch File & Folder Transfer**: 4MB TCP buffers, 256KB streaming chunks, drag-and-drop multiple files & nested folder trees onto the PC window, SAF multi-folder & multi-file picker on phone, and one-tap **Open Received Downloads Folder** (`Downloads/I_Connect_Transfers`).
5. **One-Time Device Pairing Whitelist & Auto-Discovery**: UDP broadcast + mobile hotspot gateway/subnet probing (52199), approval prompt on first connect, silent instant reconnect afterward.
6. **Windows System Tray, Auto-Start & Native Installer/Uninstaller**: Closing the PC dashboard minimizes silently to the tray; `--minimized` startup option; full registry + shortcut + firewall setup and clean uninstallation.

---

## Building Everything from Scratch
```powershell
powershell -ExecutionPolicy Bypass -File scripts\build_all.ps1
```
