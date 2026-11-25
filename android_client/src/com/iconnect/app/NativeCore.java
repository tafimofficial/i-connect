package com.iconnect.app;

public class NativeCore {
    static {
        System.loadLibrary("iconnect_core");
    }

    public static native void init(String deviceId, String deviceName);
    public static native String discoverServer();
    public static native int connectControl(String ip, int port);
    public static native void disconnect();
    public static native boolean isConnected();

    // Mouse & Touchpad
    public static native void sendMouseMove(int dx, int dy);
    public static native void sendMouseClick(int button, int action);
    public static native void sendMouseScroll(int deltaY, int deltaX);

    // Keyboard & Text
    public static native void sendKeyEvent(int vkCode, int action, int modifiers);
    public static native void sendTextInput(String text);

    // Media
    public static native void sendMediaCmd(int action, int value);

    // System Shortcuts
    public static native void sendSysCmd(int cmd);

    // High-speed Wire-Speed File Transfer
    public static native int connectFileChannel(String ip, int port);
    public static native int sendFilesBatch(String[] filePaths, FileTransferCallback callback);
    public static native int sendFilesBatchWithNames(String[] filePaths, String[] relativeNames, FileTransferCallback callback);
    public static native void listenIncomingFiles(String saveDir, FileTransferCallback callback);
}
