package com.iconnect.app;

import android.Manifest;
import android.app.Activity;
import android.app.AlertDialog;
import android.app.DownloadManager;
import android.content.ClipData;
import android.content.Context;
import android.content.DialogInterface;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.graphics.Color;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.os.StrictMode;
import android.provider.DocumentsContract;
import android.provider.OpenableColumns;
import android.text.Editable;
import android.text.TextWatcher;
import android.view.HapticFeedbackConstants;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.view.Window;
import android.view.inputmethod.InputMethodManager;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.SeekBar;
import android.widget.TextView;
import android.widget.Toast;

import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.NetworkInterface;
import java.net.Socket;
import java.util.ArrayList;
import java.util.Enumeration;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;

public class MainActivity extends Activity implements FileTransferCallback {

    private static final int REQ_PICK_FILES = 1001;
    private static final int REQ_PICK_FOLDER = 1002;
    private static final String PREFS_NAME = "i_connect_prefs";

    private static class QueuedFolder {
        Uri treeUri;
        String name;
        QueuedFolder(Uri treeUri, String name) {
            this.treeUri = treeUri;
            this.name = name;
        }
    }
    private final List<QueuedFolder> queuedFolders = new ArrayList<QueuedFolder>();

    private TextView tvStatus, tvServerIp, tvSpeed, tvFileTransferInfo, tvMediaSeekPct;
    private View dotStatus;
    private EditText etManualIp, etTextInput;
    private Button btnConnect, btnDiscover;
    private LinearLayout tabTouchpad, tabKeyboard, tabMedia, tabCommands, tabFiles;
    private TextView tabTxtTouchpad, tabTxtKeyboard, tabTxtMedia, tabTxtCommands, tabTxtFiles;
    private View panelTouchpad, panelKeyboard, panelMedia, panelCommands, panelFiles;
    private ProgressBar pbTransfer;
    private SeekBar sbSensitivity, sbMediaSeek;

    private boolean isCtrlActive = false;
    private boolean isShiftActive = false;
    private boolean isAltActive = false;
    private boolean isWinActive = false;
    private Button btnCtrl, btnShift, btnAlt, btnWin;

    private Handler mainHandler = new Handler(Looper.getMainLooper());
    private String deviceUuid;
    private String deviceName;
    private String currentServerIp = "192.168.1.100";
    private boolean isConnected = false;
    private boolean isInternalTextClearing = false;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
            Window w = getWindow();
            w.setStatusBarColor(Color.BLACK);
            w.setNavigationBarColor(Color.BLACK);
        }
        setContentView(R.layout.activity_main);

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 101);
            }
        }

        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        deviceUuid = prefs.getString("device_uuid", null);
        if (deviceUuid == null) {
            deviceUuid = UUID.randomUUID().toString();
            prefs.edit().putString("device_uuid", deviceUuid).apply();
        }

        deviceName = Build.MANUFACTURER + " " + Build.MODEL;
        NativeCore.init(deviceUuid, deviceName);

        initViews();
        setupTabs();
        setupTouchpad();
        setupKeyboard();
        setupMedia();
        setupCommands();
        setupFiles();

        triggerAutoDiscover();
    }

    @Override
    protected void onResume() {
        super.onResume();
        RemoteService.setUiCallback(this);
        RemoteService.setStateListener(new RemoteService.StateListener() {
            @Override
            public void onDisconnected() {
                mainHandler.post(new Runnable() {
                    @Override
                    public void run() {
                        isConnected = false;
                        if (tvStatus != null) tvStatus.setText("Disconnected");
                        if (dotStatus != null) dotStatus.setBackgroundResource(R.drawable.dot_red);
                        if (btnConnect != null) btnConnect.setText("Connect");
                    }
                });
            }
        });

        if (RemoteService.isRunning() || NativeCore.isConnected()) {
            isConnected = true;
            if (tvStatus != null) tvStatus.setText("CONNECTED [TURBO]");
            if (dotStatus != null) dotStatus.setBackgroundResource(R.drawable.dot_green);
            if (btnConnect != null) btnConnect.setText("Disconnect");
            String ip = RemoteService.getConnectedIp();
            if (ip != null && !ip.isEmpty()) {
                currentServerIp = ip;
                if (tvServerIp != null) tvServerIp.setText("PC: " + ip);
            }
        }
    }

    @Override
    protected void onPause() {
        super.onPause();
        RemoteService.setUiCallback(null);
    }

    private void initViews() {
        tvStatus = findViewById(R.id.tv_status);
        dotStatus = findViewById(R.id.dot_status);
        tvServerIp = findViewById(R.id.tv_server_ip);
        etManualIp = findViewById(R.id.et_manual_ip);
        btnConnect = findViewById(R.id.btn_connect);

        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        String lastIp = prefs.getString("last_server_ip", "");
        if (!lastIp.isEmpty()) {
            etManualIp.setText(lastIp);
        }
        btnDiscover = findViewById(R.id.btn_discover);

        tabTouchpad = findViewById(R.id.tab_btn_touchpad);
        tabKeyboard = findViewById(R.id.tab_btn_keyboard);
        tabMedia = findViewById(R.id.tab_btn_media);
        tabCommands = findViewById(R.id.tab_btn_commands);
        tabFiles = findViewById(R.id.tab_btn_files);

        tabTxtTouchpad = findViewById(R.id.tab_txt_touchpad);
        tabTxtKeyboard = findViewById(R.id.tab_txt_keyboard);
        tabTxtMedia = findViewById(R.id.tab_txt_media);
        tabTxtCommands = findViewById(R.id.tab_txt_commands);
        tabTxtFiles = findViewById(R.id.tab_txt_files);

        panelTouchpad = findViewById(R.id.panel_touchpad);
        panelKeyboard = findViewById(R.id.panel_keyboard);
        panelMedia = findViewById(R.id.panel_media);
        panelCommands = findViewById(R.id.panel_commands);
        panelFiles = findViewById(R.id.panel_files);

        btnConnect.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                if (isConnected) {
                    disconnectFromServer();
                } else {
                    String ip = etManualIp.getText().toString().trim();
                    if (!ip.isEmpty()) {
                        connectToServer(ip);
                    }
                }
            }
        });

        btnDiscover.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                triggerAutoDiscover();
            }
        });
    }

    private void setupTabs() {
        View.OnClickListener tabListener = new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                panelTouchpad.setVisibility(View.GONE);
                panelKeyboard.setVisibility(View.GONE);
                panelMedia.setVisibility(View.GONE);
                panelCommands.setVisibility(View.GONE);
                panelFiles.setVisibility(View.GONE);

                resetTabStyles();

                int id = v.getId();
                if (id == R.id.tab_btn_touchpad) {
                    panelTouchpad.setVisibility(View.VISIBLE);
                    tabTouchpad.setBackgroundColor(0xFF18260E);
                    tabTxtTouchpad.setTextColor(0xFFB6FF00);
                } else if (id == R.id.tab_btn_keyboard) {
                    panelKeyboard.setVisibility(View.VISIBLE);
                    tabKeyboard.setBackgroundColor(0xFF18260E);
                    tabTxtKeyboard.setTextColor(0xFFB6FF00);
                } else if (id == R.id.tab_btn_media) {
                    panelMedia.setVisibility(View.VISIBLE);
                    tabMedia.setBackgroundColor(0xFF18260E);
                    tabTxtMedia.setTextColor(0xFFB6FF00);
                } else if (id == R.id.tab_btn_commands) {
                    panelCommands.setVisibility(View.VISIBLE);
                    tabCommands.setBackgroundColor(0xFF18260E);
                    tabTxtCommands.setTextColor(0xFFB6FF00);
                } else if (id == R.id.tab_btn_files) {
                    panelFiles.setVisibility(View.VISIBLE);
                    tabFiles.setBackgroundColor(0xFF18260E);
                    tabTxtFiles.setTextColor(0xFFB6FF00);
                }
            }
        };

        tabTouchpad.setOnClickListener(tabListener);
        tabKeyboard.setOnClickListener(tabListener);
        tabMedia.setOnClickListener(tabListener);
        tabCommands.setOnClickListener(tabListener);
        tabFiles.setOnClickListener(tabListener);

        tabTouchpad.performClick();
    }

    private void resetTabStyles() {
        tabTouchpad.setBackgroundColor(0x00000000);
        tabKeyboard.setBackgroundColor(0x00000000);
        tabMedia.setBackgroundColor(0x00000000);
        tabCommands.setBackgroundColor(0x00000000);
        tabFiles.setBackgroundColor(0x00000000);

        tabTxtTouchpad.setTextColor(0xFF889A78);
        tabTxtKeyboard.setTextColor(0xFF889A78);
        tabTxtMedia.setTextColor(0xFF889A78);
        tabTxtCommands.setTextColor(0xFF889A78);
        tabTxtFiles.setTextColor(0xFF889A78);
    }

    private void triggerAutoDiscover() {
        tvStatus.setText("Scanning Wi-Fi / Hotspot...");
        new Thread(new Runnable() {
            @Override
            public void run() {
                final String foundIp = findServerIp();
                mainHandler.post(new Runnable() {
                    @Override
                    public void run() {
                        if (foundIp != null && !foundIp.isEmpty()) {
                            etManualIp.setText(foundIp);
                            Toast.makeText(MainActivity.this, "PC found at " + foundIp, Toast.LENGTH_SHORT).show();
                            connectToServer(foundIp);
                        } else {
                            tvStatus.setText("Offline (Tap Connect or Scan)");
                            Toast.makeText(MainActivity.this, "Check PC window for IP (e.g. 192.168.43.x)", Toast.LENGTH_LONG).show();
                        }
                    }
                });
            }
        }).start();
    }

    private String findServerIp() {
        SharedPreferences prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        String lastIp = prefs.getString("last_server_ip", "");
        if (!lastIp.isEmpty() && isPortOpen(lastIp, 52200, 150)) {
            return lastIp;
        }

        String beaconIp = listenForServerBeacon(900);
        if (beaconIp != null && !beaconIp.isEmpty()) {
            return beaconIp;
        }

        String found = NativeCore.discoverServer();
        if (found != null && !found.isEmpty()) {
            return found;
        }

        List<String> prefixes = getLocalSubnetPrefixes();
        final String[] result = new String[1];
        ExecutorService pool = Executors.newFixedThreadPool(16);

        for (String prefix : prefixes) {
            for (int i = 2; i <= 45; i++) {
                final String probeIp = prefix + i;
                pool.execute(new Runnable() {
                    @Override
                    public void run() {
                        if (result[0] == null && isPortOpen(probeIp, 52200, 200)) {
                            result[0] = probeIp;
                        }
                    }
                });
            }
        }

        pool.shutdown();
        try {
            pool.awaitTermination(1600, TimeUnit.MILLISECONDS);
        } catch (InterruptedException ignored) {}

        return result[0];
    }

    private String listenForServerBeacon(int timeoutMs) {
        DatagramSocket sock = null;
        try {
            sock = new DatagramSocket(52199);
            sock.setBroadcast(true);
            sock.setSoTimeout(timeoutMs);
            byte[] buf = new byte[512];
            DatagramPacket pkt = new DatagramPacket(buf, buf.length);
            sock.receive(pkt);
            String msg = new String(pkt.getData(), 0, pkt.getLength());
            if (msg.contains("RCP_SERVER_ANNOUNCE")) {
                return pkt.getAddress().getHostAddress();
            }
        } catch (Exception ignored) {
        } finally {
            if (sock != null) {
                try { sock.close(); } catch (Exception ignored) {}
            }
        }
        return null;
    }

    private List<String> getLocalSubnetPrefixes() {
        List<String> prefixes = new ArrayList<String>();
        try {
            Enumeration<NetworkInterface> interfaces = NetworkInterface.getNetworkInterfaces();
            while (interfaces != null && interfaces.hasMoreElements()) {
                NetworkInterface iface = interfaces.nextElement();
                Enumeration<InetAddress> addrs = iface.getInetAddresses();
                while (addrs.hasMoreElements()) {
                    InetAddress addr = addrs.nextElement();
                    if (!addr.isLoopbackAddress() && addr instanceof Inet4Address) {
                        String ip = addr.getHostAddress();
                        int lastDot = ip.lastIndexOf('.');
                        if (lastDot > 0) {
                            String p = ip.substring(0, lastDot + 1);
                            if (!prefixes.contains(p)) {
                                prefixes.add(p);
                            }
                        }
                    }
                }
            }
        } catch (Exception ignored) {}

        if (!prefixes.contains("192.168.43.")) {
            prefixes.add("192.168.43.");
        }
        return prefixes;
    }

    private static boolean isPortOpen(String ip, int port, int timeoutMs) {
        try {
            Socket s = new Socket();
            s.connect(new InetSocketAddress(ip, port), timeoutMs);
            s.close();
            return true;
        } catch (Exception e) {
            return false;
        }
    }

    private void connectToServer(final String ip) {
        tvStatus.setText("Connecting to " + ip + "...");
        btnConnect.setEnabled(false);

        new Thread(new Runnable() {
            @Override
            public void run() {
                final int result = NativeCore.connectControl(ip, 52200);

                mainHandler.post(new Runnable() {
                    @Override
                    public void run() {
                        btnConnect.setEnabled(true);
                        if (result == 1) {
                            isConnected = true;
                            currentServerIp = ip;
                            getSharedPreferences(PREFS_NAME, MODE_PRIVATE).edit().putString("last_server_ip", ip).apply();
                            tvStatus.setText("CONNECTED [TURBO]");
                            dotStatus.setBackgroundResource(R.drawable.dot_green);
                            tvServerIp.setText("PC: " + ip);
                            btnConnect.setText("Disconnect");
                            Toast.makeText(MainActivity.this, "Connected to PC!", Toast.LENGTH_SHORT).show();

                            RemoteService.start(MainActivity.this, ip);
                        } else if (result == 0) {
                            tvStatus.setText("Access Denied by PC");
                            dotStatus.setBackgroundResource(R.drawable.dot_red);
                            Toast.makeText(MainActivity.this, "Connection was rejected on PC.", Toast.LENGTH_LONG).show();
                        } else {
                            tvStatus.setText("Connection Failed");
                            dotStatus.setBackgroundResource(R.drawable.dot_red);
                            Toast.makeText(MainActivity.this, "Could not reach PC. Check Wi-Fi or Hotspot IP.", Toast.LENGTH_SHORT).show();
                        }
                    }
                });
            }
        }).start();
    }

    private void disconnectFromServer() {
        RemoteService.stop(this);
        NativeCore.disconnect();
        isConnected = false;
        tvStatus.setText("Disconnected");
        dotStatus.setBackgroundResource(R.drawable.dot_red);
        btnConnect.setText("Connect");
        Toast.makeText(this, "Disconnected from PC", Toast.LENGTH_SHORT).show();
    }

    private void setupTouchpad() {
        final TouchpadView tpView = findViewById(R.id.touchpad_view);
        final Button btnLeftClick = findViewById(R.id.btn_left_click);
        final Button btnRightClick = findViewById(R.id.btn_right_click);
        sbSensitivity = findViewById(R.id.sb_sensitivity);

        btnLeftClick.setOnTouchListener(new View.OnTouchListener() {
            @Override
            public boolean onTouch(View v, MotionEvent event) {
                int act = event.getActionMasked();
                if (act == MotionEvent.ACTION_DOWN) {
                    v.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY);
                    NativeCore.sendMouseClick(1, 0);
                    btnLeftClick.setPressed(true);
                    return true;
                } else if (act == MotionEvent.ACTION_UP || act == MotionEvent.ACTION_CANCEL) {
                    NativeCore.sendMouseClick(1, 1);
                    btnLeftClick.setPressed(false);
                    return true;
                }
                return false;
            }
        });

        btnRightClick.setOnTouchListener(new View.OnTouchListener() {
            @Override
            public boolean onTouch(View v, MotionEvent event) {
                int act = event.getActionMasked();
                if (act == MotionEvent.ACTION_DOWN) {
                    v.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY);
                    NativeCore.sendMouseClick(2, 0);
                    btnRightClick.setPressed(true);
                    return true;
                } else if (act == MotionEvent.ACTION_UP || act == MotionEvent.ACTION_CANCEL) {
                    NativeCore.sendMouseClick(2, 1);
                    btnRightClick.setPressed(false);
                    return true;
                }
                return false;
            }
        });

        sbSensitivity.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            @Override
            public void onProgressChanged(SeekBar seekBar, int progress, boolean fromUser) {
                float sens = 0.5f + (progress / 100.0f) * 2.5f;
                tpView.setSensitivity(sens);
            }
            @Override public void onStartTrackingTouch(SeekBar seekBar) {}
            @Override public void onStopTrackingTouch(SeekBar seekBar) {}
        });
    }

    private void setupKeyboard() {
        btnCtrl = findViewById(R.id.btn_key_ctrl);
        btnShift = findViewById(R.id.btn_key_shift);
        btnAlt = findViewById(R.id.btn_key_alt);
        btnWin = findViewById(R.id.btn_key_win);

        btnCtrl.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                isCtrlActive = !isCtrlActive;
                btnCtrl.setSelected(isCtrlActive);
                NativeCore.sendKeyEvent(0x11, isCtrlActive ? 0 : 1, 0);
            }
        });

        btnShift.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                isShiftActive = !isShiftActive;
                btnShift.setSelected(isShiftActive);
                NativeCore.sendKeyEvent(0x10, isShiftActive ? 0 : 1, 0);
            }
        });

        btnAlt.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                isAltActive = !isAltActive;
                btnAlt.setSelected(isAltActive);
                NativeCore.sendKeyEvent(0x12, isAltActive ? 0 : 1, 0);
            }
        });

        btnWin.setOnClickListener(new View.OnClickListener() {
            @Override public void onClick(View v) {
                isWinActive = !isWinActive;
                btnWin.setSelected(isWinActive);
                NativeCore.sendKeyEvent(0x5B, isWinActive ? 0 : 1, 0);
            }
        });

        bindKey(R.id.btn_key_esc, 0x1B);
        bindKey(R.id.btn_key_f1, 0x70);
        bindKey(R.id.btn_key_f2, 0x71);
        bindKey(R.id.btn_key_f3, 0x72);
        bindKey(R.id.btn_key_f4, 0x73);
        bindKey(R.id.btn_key_f5, 0x74);
        bindKey(R.id.btn_key_f6, 0x75);
        bindKey(R.id.btn_key_f7, 0x76);
        bindKey(R.id.btn_key_f8, 0x77);
        bindKey(R.id.btn_key_f9, 0x78);
        bindKey(R.id.btn_key_f10, 0x79);
        bindKey(R.id.btn_key_f11, 0x7A);
        bindKey(R.id.btn_key_f12, 0x7B);

        bindKey(R.id.btn_key_tab, 0x09);
        bindKey(R.id.btn_key_enter, 0x0D);
        bindKey(R.id.btn_key_backspace, 0x08);
        bindKey(R.id.btn_key_space, 0x20);
        bindKey(R.id.btn_key_del, 0x2E);

        bindKey(R.id.btn_key_up, 0x26);
        bindKey(R.id.btn_key_down, 0x28);
        bindKey(R.id.btn_key_left, 0x25);
        bindKey(R.id.btn_key_right, 0x27);

        bindMediaBtn(R.id.btn_kbd_vol_down, 5);
        bindMediaBtn(R.id.btn_kbd_vol_mute, 6);
        bindMediaBtn(R.id.btn_kbd_vol_up, 4);

        etTextInput = findViewById(R.id.et_keyboard_text);
        Button btnClearText = findViewById(R.id.btn_clear_text);
        if (btnClearText != null) {
            btnClearText.setOnClickListener(new View.OnClickListener() {
                @Override public void onClick(View v) {
                    isInternalTextClearing = true;
                    etTextInput.setText("");
                    isInternalTextClearing = false;
                }
            });
        }

        etTextInput.addTextChangedListener(new TextWatcher() {
            @Override public void beforeTextChanged(CharSequence s, int start, int count, int after) {}

            @Override
            public void onTextChanged(CharSequence s, int start, int before, int count) {
                if (isInternalTextClearing) return;

                if (count > before) {
                    CharSequence added = s.subSequence(start, start + count);
                    String addedStr = added.toString();
                    if (addedStr.equals("\n")) {
                        NativeCore.sendKeyEvent(0x0D, 2, 0);
                    } else {
                        NativeCore.sendTextInput(addedStr);
                    }
                } else if (before > count) {
                    int delCount = before - count;
                    for (int i = 0; i < delCount; i++) {
                        NativeCore.sendKeyEvent(0x08, 2, 0);
                    }
                }
            }

            @Override
            public void afterTextChanged(Editable s) {
                if (s.length() > 60) {
                    isInternalTextClearing = true;
                    s.clear();
                    isInternalTextClearing = false;
                }
            }
        });

        etTextInput.setOnKeyListener(new View.OnKeyListener() {
            @Override
            public boolean onKey(View v, int keyCode, KeyEvent event) {
                if (event.getAction() == KeyEvent.ACTION_DOWN) {
                    if (keyCode == KeyEvent.KEYCODE_DEL) {
                        if (etTextInput.getText().length() == 0) {
                            NativeCore.sendKeyEvent(0x08, 2, 0);
                            return true;
                        }
                    } else if (keyCode == KeyEvent.KEYCODE_ENTER) {
                        NativeCore.sendKeyEvent(0x0D, 2, 0);
                        isInternalTextClearing = true;
                        etTextInput.setText("");
                        isInternalTextClearing = false;
                        return true;
                    }
                }
                return false;
            }
        });

        Button btnSoftKeyboard = findViewById(R.id.btn_toggle_softkbd);
        btnSoftKeyboard.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                etTextInput.requestFocus();
                InputMethodManager imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
                if (imm != null) {
                    imm.showSoftInput(etTextInput, InputMethodManager.SHOW_IMPLICIT);
                }
            }
        });
    }

    private void bindKey(int viewId, final int vkCode) {
        View v = findViewById(viewId);
        if (v != null) {
            v.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View view) {
                    view.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY);
                    int mods = (isShiftActive ? 1 : 0) | (isCtrlActive ? 2 : 0) | (isAltActive ? 4 : 0) | (isWinActive ? 8 : 0);
                    NativeCore.sendKeyEvent(vkCode, 2, mods);
                }
            });
        }
    }

    private void bindMediaBtn(int viewId, final int action) {
        View v = findViewById(viewId);
        if (v != null) {
            v.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View view) {
                    view.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY);
                    NativeCore.sendMediaCmd(action, 0);
                }
            });
        }
    }

    private void setupMedia() {
        bindMediaBtn(R.id.btn_media_play_pause, 1);
        bindMediaBtn(R.id.btn_media_prev, 2);
        bindMediaBtn(R.id.btn_media_next, 3);
        bindMediaBtn(R.id.btn_media_vol_up, 4);
        bindMediaBtn(R.id.btn_media_vol_down, 5);
        bindMediaBtn(R.id.btn_media_mute, 6);
        bindMediaBtn(R.id.btn_media_seek_back, 7);
        bindMediaBtn(R.id.btn_media_seek_fwd, 8);

        tvMediaSeekPct = findViewById(R.id.tv_media_seek_pct);
        sbMediaSeek = findViewById(R.id.sb_media_seek);
        if (sbMediaSeek != null) {
            sbMediaSeek.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
                @Override
                public void onProgressChanged(SeekBar seekBar, int progress, boolean fromUser) {
                    if (tvMediaSeekPct != null) {
                        tvMediaSeekPct.setText(progress + "%");
                    }
                }
                @Override public void onStartTrackingTouch(SeekBar seekBar) {}
                @Override
                public void onStopTrackingTouch(SeekBar seekBar) {
                    NativeCore.sendMediaCmd(9, seekBar.getProgress());
                }
            });
        }
    }

    private void setupCommands() {
        bindSysCmd(R.id.btn_cmd_lock, 1, false);
        bindSysCmd(R.id.btn_cmd_sleep, 2, false);
        bindSysCmd(R.id.btn_cmd_shutdown, 3, true);
        bindSysCmd(R.id.btn_cmd_restart, 4, true);
        bindSysCmd(R.id.btn_cmd_showdesktop, 5, false);
        bindSysCmd(R.id.btn_cmd_closewindow, 6, false);
        bindSysCmd(R.id.btn_cmd_taskmgr, 7, false);
    }

    private void bindSysCmd(int viewId, final int cmdCode, final boolean requireConfirm) {
        View v = findViewById(viewId);
        if (v != null) {
            v.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View view) {
                    view.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY);
                    if (requireConfirm) {
                        String title = (cmdCode == 3) ? "Shutdown PC?" : "Restart PC?";
                        new AlertDialog.Builder(MainActivity.this)
                                .setTitle(title)
                                .setMessage("Are you sure you want to perform this system action on your PC?")
                                .setPositiveButton("Yes", new DialogInterface.OnClickListener() {
                                    @Override
                                    public void onClick(DialogInterface dialog, int which) {
                                        NativeCore.sendSysCmd(cmdCode);
                                    }
                                })
                                .setNegativeButton("Cancel", null)
                                .show();
                    } else {
                        NativeCore.sendSysCmd(cmdCode);
                    }
                }
            });
        }
    }

    private void setupFiles() {
        Button btnPickFolders = findViewById(R.id.btn_pick_folders);
        Button btnPickFiles = findViewById(R.id.btn_pick_files);
        Button btnOpenDownload = findViewById(R.id.btn_open_downloads);
        pbTransfer = findViewById(R.id.pb_file_transfer);
        tvSpeed = findViewById(R.id.tv_transfer_speed);
        tvFileTransferInfo = findViewById(R.id.tv_transfer_info);

        btnPickFolders.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                if (!isConnected) {
                    Toast.makeText(MainActivity.this, "Please connect to PC first!", Toast.LENGTH_SHORT).show();
                    return;
                }
                queuedFolders.clear();
                openFolderPicker();
            }
        });

        btnPickFiles.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                if (!isConnected) {
                    Toast.makeText(MainActivity.this, "Please connect to PC first!", Toast.LENGTH_SHORT).show();
                    return;
                }
                Intent intent = new Intent(Intent.ACTION_GET_CONTENT);
                intent.setType("*/*");
                intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
                startActivityForResult(Intent.createChooser(intent, "Select Files to Send"), REQ_PICK_FILES);
            }
        });

        btnOpenDownload.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                openReceivedFolder();
            }
        });
    }

    private void openReceivedFolder() {
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
        if (!saveFolder.exists()) {
            saveFolder.mkdirs();
        }

        boolean success = false;

        try {
            Uri uri = Uri.parse("content://com.android.externalstorage.documents/document/primary%3ADownload%2FI_Connect_Transfers");
            Intent intent = new Intent(Intent.ACTION_VIEW);
            intent.setDataAndType(uri, DocumentsContract.Document.MIME_TYPE_DIR);
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_GRANT_READ_URI_PERMISSION);
            startActivity(intent);
            success = true;
        } catch (Exception ignored) {}

        if (!success) {
            try {
                Intent intent = new Intent(DownloadManager.ACTION_VIEW_DOWNLOADS);
                intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                startActivity(intent);
                success = true;
            } catch (Exception ignored) {}
        }

        if (!success) {
            try {
                StrictMode.VmPolicy.Builder builder = new StrictMode.VmPolicy.Builder();
                StrictMode.setVmPolicy(builder.build());
                Uri uri = Uri.fromFile(saveFolder);
                Intent intent = new Intent(Intent.ACTION_VIEW);
                intent.setDataAndType(uri, "resource/folder");
                intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                startActivity(Intent.createChooser(intent, "Open Received Folder"));
                success = true;
            } catch (Exception ignored) {}
        }

        if (!success) {
            try {
                Intent intent = new Intent(Intent.ACTION_GET_CONTENT);
                Uri uri = Uri.fromFile(saveFolder);
                intent.setDataAndType(uri, "*/*");
                intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                startActivity(Intent.createChooser(intent, "Open Received Folder"));
            } catch (Exception ignored) {}
        }

        Toast.makeText(this, "Opening: " + saveFolder.getAbsolutePath(), Toast.LENGTH_SHORT).show();
    }

    private void openFolderPicker() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        startActivityForResult(intent, REQ_PICK_FOLDER);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == REQ_PICK_FOLDER && resultCode == RESULT_OK && data != null) {
            Uri treeUri = data.getData();
            if (treeUri != null) {
                try {
                    getContentResolver().takePersistableUriPermission(treeUri,
                            Intent.FLAG_GRANT_READ_URI_PERMISSION);
                } catch (Exception ignored) {}

                String folderName = "Folder";
                try {
                    Uri docUri = DocumentsContract.buildDocumentUriUsingTree(treeUri,
                            DocumentsContract.getTreeDocumentId(treeUri));
                    Cursor cursor = getContentResolver().query(docUri,
                            new String[]{DocumentsContract.Document.COLUMN_DISPLAY_NAME}, null, null, null);
                    if (cursor != null) {
                        if (cursor.moveToFirst()) {
                            int idx = cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DISPLAY_NAME);
                            if (idx >= 0) folderName = cursor.getString(idx);
                        }
                        cursor.close();
                    }
                } catch (Exception e) {
                    e.printStackTrace();
                }

                queuedFolders.add(new QueuedFolder(treeUri, folderName));
                showAddMoreFoldersDialog();
            }
        } else if (requestCode == REQ_PICK_FILES && resultCode == RESULT_OK && data != null) {
            final ArrayList<Uri> pickedUris = new ArrayList<Uri>();
            if (data.getClipData() != null) {
                ClipData clipData = data.getClipData();
                for (int i = 0; i < clipData.getItemCount(); i++) {
                    pickedUris.add(clipData.getItemAt(i).getUri());
                }
            } else if (data.getData() != null) {
                pickedUris.add(data.getData());
            }

            if (!pickedUris.isEmpty()) {
                tvFileTransferInfo.setText("Preparing zero-copy turbo stream (" + pickedUris.size() + " files)...");
                pbTransfer.setVisibility(View.VISIBLE);
                pbTransfer.setProgress(0);

                new Thread(new Runnable() {
                    @Override
                    public void run() {
                        ArrayList<Integer> fds = new ArrayList<Integer>();
                        ArrayList<String> names = new ArrayList<String>();
                        ArrayList<Long> sizes = new ArrayList<Long>();
                        ArrayList<File> tempCleanup = new ArrayList<File>();

                        for (Uri uri : pickedUris) {
                            appendUriFd(uri, null, fds, names, sizes, tempCleanup);
                        }

                        dispatchFdBatch(fds, names, sizes, tempCleanup);
                    }
                }).start();
            }
        }
    }

    private void showAddMoreFoldersDialog() {
        StringBuilder sb = new StringBuilder();
        sb.append("Queued Folders (").append(queuedFolders.size()).append("):\n");
        for (int i = 0; i < queuedFolders.size(); i++) {
            sb.append("• ").append(queuedFolders.get(i).name).append("\n");
        }
        sb.append("\nYou can add more folders or start sending to PC now.");

        new AlertDialog.Builder(MainActivity.this)
                .setTitle("📁 Folders to Send")
                .setMessage(sb.toString())
                .setPositiveButton("🚀 Send Now (" + queuedFolders.size() + ")", new DialogInterface.OnClickListener() {
                    @Override
                    public void onClick(DialogInterface dialog, int which) {
                        startSendingFolders();
                    }
                })
                .setNeutralButton("➕ Add Another Folder", new DialogInterface.OnClickListener() {
                    @Override
                    public void onClick(DialogInterface dialog, int which) {
                        openFolderPicker();
                    }
                })
                .setNegativeButton("Cancel", new DialogInterface.OnClickListener() {
                    @Override
                    public void onClick(DialogInterface dialog, int which) {
                        queuedFolders.clear();
                    }
                })
                .setCancelable(false)
                .show();
    }

    private void startSendingFolders() {
        if (queuedFolders.isEmpty()) return;

        final List<QueuedFolder> foldersToSend = new ArrayList<QueuedFolder>(queuedFolders);
        queuedFolders.clear();

        tvFileTransferInfo.setText("Scanning " + foldersToSend.size() + " folder(s) for zero-copy turbo send...");
        pbTransfer.setVisibility(View.VISIBLE);
        pbTransfer.setProgress(0);

        new Thread(new Runnable() {
            @Override
            public void run() {
                final ArrayList<Integer> fds = new ArrayList<Integer>();
                final ArrayList<String> relNames = new ArrayList<String>();
                final ArrayList<Long> sizes = new ArrayList<Long>();
                final ArrayList<File> tempCleanup = new ArrayList<File>();

                for (QueuedFolder qf : foldersToSend) {
                    try {
                        String rootDocId = DocumentsContract.getTreeDocumentId(qf.treeUri);
                        traverseDocumentTreeFds(qf.treeUri, rootDocId, qf.name, fds, relNames, sizes, tempCleanup);
                    } catch (Exception e) {
                        e.printStackTrace();
                    }
                }

                if (fds.isEmpty()) {
                    mainHandler.post(new Runnable() {
                        @Override
                        public void run() {
                            tvFileTransferInfo.setText("No files found in selected folder(s).");
                            pbTransfer.setVisibility(View.GONE);
                            Toast.makeText(MainActivity.this, "No files found inside folder(s)!", Toast.LENGTH_SHORT).show();
                        }
                    });
                    return;
                }

                dispatchFdBatch(fds, relNames, sizes, tempCleanup);
            }
        }).start();
    }

    private void dispatchFdBatch(final ArrayList<Integer> fds,
                                 final ArrayList<String> names,
                                 final ArrayList<Long> sizes,
                                 final ArrayList<File> tempCleanup) {
        try {
            if (fds.isEmpty()) {
                mainHandler.post(new Runnable() {
                    @Override
                    public void run() {
                        tvFileTransferInfo.setText("No readable files selected.");
                        pbTransfer.setVisibility(View.GONE);
                        Toast.makeText(MainActivity.this, "Could not open selected file(s)!", Toast.LENGTH_SHORT).show();
                    }
                });
                return;
            }

            final int count = fds.size();
            mainHandler.post(new Runnable() {
                @Override
                public void run() {
                    tvFileTransferInfo.setText("Turbo streaming " + count + " file(s) (4x zero-copy)...");
                }
            });

            int[] fdArr = new int[count];
            String[] nameArr = new String[count];
            long[] sizeArr = new long[count];
            for (int i = 0; i < count; i++) {
                fdArr[i] = fds.get(i);
                nameArr[i] = names.get(i);
                sizeArr[i] = sizes.get(i);
            }

            final int res = NativeCore.sendFilesFromFds(fdArr, nameArr, sizeArr, currentServerIp, 52201, MainActivity.this);
            if (res < 0) {
                mainHandler.post(new Runnable() {
                    @Override
                    public void run() {
                        pbTransfer.setVisibility(View.GONE);
                        tvFileTransferInfo.setText("Transfer failed (err " + res + "). Check PC connection.");
                        Toast.makeText(MainActivity.this, "Turbo send failed (" + res + ")", Toast.LENGTH_SHORT).show();
                    }
                });
            }
        } finally {
            for (File f : tempCleanup) {
                try { f.delete(); } catch (Exception ignored) {}
            }
        }
    }

    private void appendUriFd(Uri uri, String explicitRelName,
                             List<Integer> outFds, List<String> outNames,
                             List<Long> outSizes, List<File> tempCleanup) {
        String displayName = explicitRelName;
        long queriedSize = -1L;

        Cursor cursor = null;
        try {
            cursor = getContentResolver().query(uri, null, null, null, null);
            if (cursor != null && cursor.moveToFirst()) {
                if (displayName == null || displayName.isEmpty()) {
                    int nameIdx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                    if (nameIdx >= 0) displayName = cursor.getString(nameIdx);
                }
                int sizeIdx = cursor.getColumnIndex(OpenableColumns.SIZE);
                if (sizeIdx >= 0 && !cursor.isNull(sizeIdx)) {
                    queriedSize = cursor.getLong(sizeIdx);
                }
            }
        } catch (Exception ignored) {
        } finally {
            if (cursor != null) {
                try { cursor.close(); } catch (Exception ignored) {}
            }
        }

        if (displayName == null || displayName.isEmpty()) {
            displayName = "file_" + System.currentTimeMillis();
        }

        ParcelFileDescriptor pfd = null;
        try {
            pfd = getContentResolver().openFileDescriptor(uri, "r");
            if (pfd != null) {
                long statSize = pfd.getStatSize();
                if (statSize >= 0) {
                    int detachedFd = pfd.detachFd();
                    pfd = null;
                    long finalSize = (statSize > 0) ? statSize : Math.max(0L, queriedSize);
                    outFds.add(detachedFd);
                    outNames.add(displayName);
                    outSizes.add(finalSize);
                    return;
                }
                pfd.close();
                pfd = null;
            }
        } catch (Exception e) {
            e.printStackTrace();
            if (pfd != null) {
                try { pfd.close(); } catch (Exception ignored) {}
                pfd = null;
            }
        }

        // Fallback only for virtual/non-seekable streams (pipes) that lack a backing file descriptor
        String[] fallback = copyUriToTempWithName(uri);
        if (fallback != null) {
            File tmp = new File(fallback[0]);
            ParcelFileDescriptor tmpPfd = null;
            try {
                tmpPfd = ParcelFileDescriptor.open(tmp, ParcelFileDescriptor.MODE_READ_ONLY);
                int detachedFd = tmpPfd.detachFd();
                tmpPfd = null;
                outFds.add(detachedFd);
                outNames.add(explicitRelName != null ? explicitRelName : fallback[1]);
                outSizes.add(tmp.length());
                tempCleanup.add(tmp);
            } catch (Exception e) {
                e.printStackTrace();
                if (tmpPfd != null) {
                    try { tmpPfd.close(); } catch (Exception ignored) {}
                }
                try { tmp.delete(); } catch (Exception ignored) {}
            }
        }
    }

    private void traverseDocumentTreeFds(Uri treeUri, String currentDocId, String currentRelPath,
                                          List<Integer> outFds, List<String> outNames,
                                          List<Long> outSizes, List<File> tempCleanup) {
        Cursor c = null;
        try {
            Uri childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, currentDocId);
            c = getContentResolver().query(childrenUri, new String[]{
                    DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                    DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                    DocumentsContract.Document.COLUMN_MIME_TYPE
            }, null, null, null);

            if (c == null) return;

            while (c.moveToNext()) {
                String docId = c.getString(0);
                String displayName = c.getString(1);
                String mimeType = c.getString(2);

                if (displayName == null) displayName = "unnamed";
                String relPath = currentRelPath.isEmpty() ? displayName : (currentRelPath + "/" + displayName);

                if (DocumentsContract.Document.MIME_TYPE_DIR.equals(mimeType)) {
                    traverseDocumentTreeFds(treeUri, docId, relPath, outFds, outNames, outSizes, tempCleanup);
                } else {
                    Uri fileUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, docId);
                    appendUriFd(fileUri, relPath, outFds, outNames, outSizes, tempCleanup);
                }
            }
        } catch (Exception e) {
            e.printStackTrace();
        } finally {
            if (c != null) {
                try { c.close(); } catch (Exception ignored) {}
            }
        }
    }

    private String[] copyUriToTempWithName(Uri uri) {
        try {
            String filename = "file_" + System.currentTimeMillis();
            Cursor cursor = getContentResolver().query(uri, null, null, null, null);
            if (cursor != null && cursor.moveToFirst()) {
                int nameIndex = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (nameIndex >= 0) filename = cursor.getString(nameIndex);
                cursor.close();
            }

            File tempFile = new File(getCacheDir(), filename);
            InputStream in = getContentResolver().openInputStream(uri);
            if (in == null) return null;
            FileOutputStream out = new FileOutputStream(tempFile);
            byte[] buf = new byte[1024 * 1024];
            int len;
            while ((len = in.read(buf)) > 0) {
                out.write(buf, 0, len);
            }
            in.close();
            out.close();
            return new String[]{ tempFile.getAbsolutePath(), filename };
        } catch (Exception e) {
            e.printStackTrace();
            return null;
        }
    }

    @Override
    public void onFileProgress(final int fileIdx, final int totalFiles, final String name, final long transferred, final long totalBytes, final float mbps) {
        final int pct = (totalBytes > 0) ? (int) ((transferred * 100) / totalBytes) : 0;
        RemoteService svc = RemoteService.getInstance();
        if (svc != null) {
            svc.updateTransferNotification(name, pct, mbps, fileIdx, totalFiles);
        }
        mainHandler.post(new Runnable() {
            @Override
            public void run() {
                if (pbTransfer != null) pbTransfer.setVisibility(View.VISIBLE);
                if (pbTransfer != null) pbTransfer.setProgress(pct);
                if (tvSpeed != null) tvSpeed.setText(String.format("%.1f MB/s", mbps));
                if (tvFileTransferInfo != null) tvFileTransferInfo.setText(String.format("[%d/%d] %s (%d%%)", fileIdx, totalFiles, name, pct));
            }
        });
    }

    @Override
    public void onBatchFinished(final int totalFiles) {
        RemoteService svc = RemoteService.getInstance();
        if (svc != null) {
            svc.updateFinishedNotification(totalFiles);
        }
        mainHandler.post(new Runnable() {
            @Override
            public void run() {
                if (pbTransfer != null) pbTransfer.setProgress(100);
                if (tvSpeed != null) tvSpeed.setText("0.0 MB/s");
                if (tvFileTransferInfo != null) tvFileTransferInfo.setText("Batch complete (" + totalFiles + " files transferred)");
                Toast.makeText(MainActivity.this, "File transfer completed successfully!", Toast.LENGTH_SHORT).show();
            }
        });
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        if (etManualIp != null && etManualIp.hasFocus()) {
            return super.dispatchKeyEvent(event);
        }
        if (etTextInput != null && etTextInput.hasFocus()) {
            return super.dispatchKeyEvent(event);
        }

        if (isConnected) {
            int keyCode = event.getKeyCode();
            if (keyCode != KeyEvent.KEYCODE_BACK &&
                keyCode != KeyEvent.KEYCODE_VOLUME_UP &&
                keyCode != KeyEvent.KEYCODE_VOLUME_DOWN &&
                keyCode != KeyEvent.KEYCODE_VOLUME_MUTE) {

                int action = event.getAction();
                if (action == KeyEvent.ACTION_DOWN || action == KeyEvent.ACTION_UP) {
                    int vk = mapAndroidKeyToWindowsVk(keyCode);
                    if (vk > 0) {
                        int jniAction = (action == KeyEvent.ACTION_DOWN) ? 0 : 1;
                        int mods = (event.isShiftPressed() ? 1 : 0) |
                                   (event.isCtrlPressed() ? 2 : 0) |
                                   (event.isAltPressed() ? 4 : 0);
                        NativeCore.sendKeyEvent(vk, jniAction, mods);
                        return true;
                    }
                }
            }
        }
        return super.dispatchKeyEvent(event);
    }

    private int mapAndroidKeyToWindowsVk(int keyCode) {
        if (keyCode >= KeyEvent.KEYCODE_A && keyCode <= KeyEvent.KEYCODE_Z) {
            return 0x41 + (keyCode - KeyEvent.KEYCODE_A);
        }
        if (keyCode >= KeyEvent.KEYCODE_0 && keyCode <= KeyEvent.KEYCODE_9) {
            return 0x30 + (keyCode - KeyEvent.KEYCODE_0);
        }
        if (keyCode >= KeyEvent.KEYCODE_F1 && keyCode <= KeyEvent.KEYCODE_F12) {
            return 0x70 + (keyCode - KeyEvent.KEYCODE_F1);
        }
        switch (keyCode) {
            case KeyEvent.KEYCODE_SPACE: return 0x20;
            case KeyEvent.KEYCODE_ENTER: return 0x0D;
            case KeyEvent.KEYCODE_DEL: return 0x08;
            case KeyEvent.KEYCODE_FORWARD_DEL: return 0x2E;
            case KeyEvent.KEYCODE_TAB: return 0x09;
            case KeyEvent.KEYCODE_ESCAPE: return 0x1B;
            case KeyEvent.KEYCODE_DPAD_UP: return 0x26;
            case KeyEvent.KEYCODE_DPAD_DOWN: return 0x28;
            case KeyEvent.KEYCODE_DPAD_LEFT: return 0x25;
            case KeyEvent.KEYCODE_DPAD_RIGHT: return 0x27;
            case KeyEvent.KEYCODE_MOVE_HOME: return 0x24;
            case KeyEvent.KEYCODE_MOVE_END: return 0x23;
            case KeyEvent.KEYCODE_PAGE_UP: return 0x21;
            case KeyEvent.KEYCODE_PAGE_DOWN: return 0x22;
            case KeyEvent.KEYCODE_SHIFT_LEFT:
            case KeyEvent.KEYCODE_SHIFT_RIGHT: return 0x10;
            case KeyEvent.KEYCODE_CTRL_LEFT:
            case KeyEvent.KEYCODE_CTRL_RIGHT: return 0x11;
            case KeyEvent.KEYCODE_ALT_LEFT:
            case KeyEvent.KEYCODE_ALT_RIGHT: return 0x12;
            case KeyEvent.KEYCODE_META_LEFT:
            case KeyEvent.KEYCODE_META_RIGHT: return 0x5B;
            default: return 0;
        }
    }

    @Override
    protected void onDestroy() {
        super.onDestroy();
        RemoteService.setUiCallback(null);
        RemoteService.setStateListener(null);
    }
}
