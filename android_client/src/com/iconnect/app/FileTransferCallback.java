package com.iconnect.app;

public interface FileTransferCallback {
    void onFileProgress(int fileIdx, int totalFiles, String name, long transferred, long totalBytes, float mbps);
    void onBatchFinished(int totalFiles);
}
