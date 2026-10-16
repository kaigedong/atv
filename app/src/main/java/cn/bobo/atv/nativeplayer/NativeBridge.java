package cn.bobo.atv.nativeplayer;

import android.view.Surface;

/** The entire Android/Rust boundary. There is no Java networking or media engine. */
public final class NativeBridge {
    private static final boolean AVAILABLE;
    static {
        boolean loaded;
        try {
            System.loadLibrary("atv_native");
            loaded = true;
        } catch (UnsatisfiedLinkError error) {
            loaded = false;
        }
        AVAILABLE = loaded;
    }

    private NativeBridge() { }
    public static boolean available() { return AVAILABLE; }
    public static native long create(Surface surface, int sampleRate, String storageDirectory);
    public static native void select(long handle, String pid, String streamId);
    public static native void pause(long handle);
    public static native void resume(long handle);
    public static native void destroy(long handle);
    public static native String status(long handle);
    public static native boolean quality(long handle, String quality);
    public static native long updateCreate(String cacheDirectory, String downloadDirectory, int version, int api);
    public static native boolean updateCommand(long handle, int action);
    public static native String updateStatus(long handle);
    public static native boolean updateVerify(long handle);
    public static native void updateDestroy(long handle);
}
