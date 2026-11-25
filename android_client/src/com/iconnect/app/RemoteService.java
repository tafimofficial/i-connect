package com.iconnect.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.Environment;
import android.os.IBinder;
import android.os.PowerManager;
import java.io.File;

public class RemoteService extends Service {

    public static final String ACTION_START = "com.iconnect.app.action.START";
    public static final String ACTION_STOP = "com.iconnect.app.action.STOP";
    public static final String ACTION_DISCONNECT = "com.iconnect.app.action.DISCONNECT";
    public static final String EXTRA_IP = "extra_ip";

    public static final String CHANNEL_ID = "iconnect_channel_v1";
    public static final int NOTIFICATION_ID = 1001;

    public interface StateListener {
        void onDisconnected();
    }

    private static RemoteService instance;
    private static FileTransferCallback activeUiCallback;
    private static StateListener stateListener;
    private static String connectedServerIp = "";
    private static boolean isRunning = false;

    private PowerManager.WakeLock wakeLock;
    private WifiManager.WifiLock wifiLock;
    private NotificationManager notificationManager;
    private boolean isListeningIncoming = false;

    public static RemoteService getInstance() {
        return instance;
    }

    public static boolean isRunning() {
        return isRunning && NativeCore.isConnected();
    }

    public static String getConnectedIp() {
        return connectedServerIp;
    }

    public static void setUiCallback(FileTransferCallback cb) {
        activeUiCallback = cb;
    }

    public static void setStateListener(StateListener l) {
        stateListener = l;
    }

    public static void start(Context context, String ip) {
        Intent intent = new Intent(context, RemoteService.class);
        intent.setAction(ACTION_START);
        intent.putExtra(EXTRA_IP, ip);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            context.startForegroundService(intent);
        } else {
            context.startService(intent);
        }
    }

    public static void stop(Context context) {
        Intent intent = new Intent(context, RemoteService.class);
        intent.setAction(ACTION_STOP);
        context.startService(intent);
    }

    @Override
    public void onCreate() {
        super.onCreate();
        instance = this;
        isRunning = true;
        notificationManager = (NotificationManager) getSystemService(Context.NOTIFICATION_SERVICE);
        createNotificationChannel();
        acquireLocks();
    }

    private void createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && notificationManager != null) {
            NotificationChannel channel = new NotificationChannel(
                    CHANNEL_ID,
                    "I Connect Connection",
                    NotificationManager.IMPORTANCE_LOW
            );
            channel.setDescription("Keeps remote control and file transfers active in the background");
            channel.setShowBadge(false);
            channel.enableLights(false);
            channel.enableVibration(false);
            notificationManager.createNotificationChannel(channel);
        }
    }

    private void acquireLocks() {
        try {
            PowerManager pm = (PowerManager) getSystemService(Context.POWER_SERVICE);
            if (pm != null && (wakeLock == null || !wakeLock.isHeld())) {
                wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "IConnect:BackgroundWakeLock");
                wakeLock.setReferenceCounted(false);
                wakeLock.acquire();
            }
        } catch (Exception e) {
            e.printStackTrace();
        }

        try {
            WifiManager wm = (WifiManager) getApplicationContext().getSystemService(Context.WIFI_SERVICE);
            if (wm != null && (wifiLock == null || !wifiLock.isHeld())) {
                wifiLock = wm.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "IConnect:WifiLock");
                wifiLock.setReferenceCounted(false);
                wifiLock.acquire();
            }
        } catch (Exception e) {
            e.printStackTrace();
        }
    }

    private void releaseLocks() {
        try {
            if (wakeLock != null && wakeLock.isHeld()) {
                wakeLock.release();
            }
        } catch (Exception ignored) {}

        try {
            if (wifiLock != null && wifiLock.isHeld()) {
                wifiLock.release();
            }
        } catch (Exception ignored) {}
    }

    private Notification buildNotification(String title, String contentText, int progress, boolean isIndeterminate, boolean showProgress) {
        Intent openIntent = new Intent(this, MainActivity.class);
        openIntent.setFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        int piFlags = (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M)
                ? (PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE)
                : PendingIntent.FLAG_UPDATE_CURRENT;
        PendingIntent piOpen = PendingIntent.getActivity(this, 0, openIntent, piFlags);

        Intent disconnectIntent = new Intent(this, RemoteService.class);
        disconnectIntent.setAction(ACTION_DISCONNECT);
        PendingIntent piDisconnect = PendingIntent.getService(this, 1, disconnectIntent, piFlags);

        Notification.Builder builder;
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            builder = new Notification.Builder(this, CHANNEL_ID);
        } else {
            builder = new Notification.Builder(this);
        }

        builder.setSmallIcon(R.mipmap.ic_launcher)
                .setContentTitle(title)
                .setContentText(contentText)
                .setContentIntent(piOpen)
                .setOngoing(true);

        if (showProgress) {
            builder.setProgress(100, progress, isIndeterminate);
        }

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.KITKAT_WATCH) {
            builder.addAction(new Notification.Action.Builder(0, "Disconnect", piDisconnect).build());
        } else {
            builder.addAction(0, "Disconnect", piDisconnect);
        }

        return builder.build();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null) {
            String action = intent.getAction();
            if (ACTION_DISCONNECT.equals(action) || ACTION_STOP.equals(action)) {
                handleDisconnect();
                return START_NOT_STICKY;
            } else if (ACTION_START.equals(action)) {
                String ip = intent.getStringExtra(EXTRA_IP);
                if (ip != null && !ip.isEmpty()) {
                    connectedServerIp = ip;
                }
                acquireLocks();
                startForeground(NOTIFICATION_ID, buildNotification(
                        "I Connect: Connected",
                        "PC: " + connectedServerIp + " (Background & Transfer Active)",
                        0, false, false));
                startIncomingFileListener(connectedServerIp);
            }
        }
        return START_STICKY;
    }

    private void handleDisconnect() {
        isRunning = false;
        NativeCore.disconnect();
        releaseLocks();
        if (stateListener != null) {
            stateListener.onDisconnected();
        }
        stopForeground(true);
        stopSelf();
    }

    @Override
    public void onTaskRemoved(Intent rootIntent) {
        // Keep ForegroundService, sockets, and active transfers running even if swiped from Recents.
    }

    @Override
    public void onDestroy() {
        super.onDestroy();
        isRunning = false;
        instance = null;
        releaseLocks();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    private long lastNotificationUpdate = 0;

    public void updateTransferNotification(String name, int pct, float mbps, int currentFile, int totalFiles) {
        long now = System.currentTimeMillis();
        if (now - lastNotificationUpdate < 350 && pct < 100) {
            return;
        }
        lastNotificationUpdate = now;

        if (notificationManager != null && isRunning) {
            String title = "Transferring (" + currentFile + "/" + totalFiles + "): " + name;
            String text = String.format("%.1f MB/s - %d%%", mbps, pct);
            notificationManager.notify(NOTIFICATION_ID, buildNotification(title, text, pct, false, true));
        }
    }

    public void updateFinishedNotification(int totalFiles) {
        if (notificationManager != null && isRunning) {
            String title = "File Transfer Complete";
            String text = "Transferred " + totalFiles + " file(s) successfully.";
            notificationManager.notify(NOTIFICATION_ID, buildNotification(title, text, 100, false, false));
        }
    }

    private synchronized void startIncomingFileListener(final String ip) {
        if (isListeningIncoming || ip == null || ip.isEmpty()) return;
        isListeningIncoming = true;

        new Thread(new Runnable() {
            @Override
            public void run() {
                try {
                    int res = NativeCore.connectFileChannel(ip, 52201);
                    if (res == 1) {
                        File saveFolder = null;
                        try {
                            File downloadDir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS);
                            if (downloadDir != null && (downloadDir.exists() || downloadDir.mkdirs())) {
                                saveFolder = new File(downloadDir, "I_Connect_Transfers");
                            }
                        } catch (Exception ignored) {}
                        if (saveFolder == null) {
                            saveFolder = new File(getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS), "I_Connect_Transfers");
                        }
                        if (!saveFolder.exists()) saveFolder.mkdirs();

                        NativeCore.listenIncomingFiles(saveFolder.getAbsolutePath(), new FileTransferCallback() {
                            @Override
                            public void onFileProgress(int fileIdx, int totalFiles, String name, long transferred, long totalBytes, float mbps) {
                                int pct = (totalBytes > 0) ? (int) ((transferred * 100) / totalBytes) : 0;
                                updateTransferNotification(name, pct, mbps, fileIdx, totalFiles);
                                if (activeUiCallback != null) {
                                    activeUiCallback.onFileProgress(fileIdx, totalFiles, name, transferred, totalBytes, mbps);
                                }
                            }

                            @Override
                            public void onBatchFinished(int totalFiles) {
                                updateFinishedNotification(totalFiles);
                                if (activeUiCallback != null) {
                                    activeUiCallback.onBatchFinished(totalFiles);
                                }
                            }
                        });
                    }
                } catch (Exception e) {
                    e.printStackTrace();
                } finally {
                    isListeningIncoming = false;
                }
            }
        }).start();
    }
}
