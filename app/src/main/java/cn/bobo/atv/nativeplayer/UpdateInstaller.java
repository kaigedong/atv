package cn.bobo.atv.nativeplayer;

import android.app.Activity;
import android.content.ClipData;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.Signature;
import android.net.Uri;
import android.os.Build;
import android.provider.Settings;
import java.io.File;
import java.io.IOException;
import java.security.MessageDigest;

/** Android package/certificate inspection and system installer handoff only. */
final class UpdateInstaller {
    static final class Failure extends Exception {
        Failure(String message) { super(message); }
    }
    private UpdateInstaller() { }
    @SuppressWarnings("deprecation")
    static File inspect(Activity activity, File directory, String relative, int version, String expectedCertificate) throws Exception {
        if (!relative.matches("slot-[0-9]+-[0-9]+/update\\.apk")) throw new Failure("更新文件路径异常");
        File root = directory.getCanonicalFile(), file = new File(root, relative).getCanonicalFile();
        if (!file.getPath().startsWith(root.getPath() + File.separator) || !file.isFile()) throw new Failure("更新文件已不存在，请重新下载");
        PackageManager pm = activity.getPackageManager();
        PackageInfo archive = pm.getPackageArchiveInfo(file.getPath(), PackageManager.GET_SIGNATURES);
        PackageInfo installed = pm.getPackageInfo(activity.getPackageName(), PackageManager.GET_SIGNATURES);
        if (archive == null || !activity.getPackageName().equals(archive.packageName)
                || archive.versionCode != version || archive.versionCode <= installed.versionCode)
            throw new Failure("安装包名称或版本不匹配，请重新检查更新");
        if (!certificate(archive.signatures).equals(expectedCertificate)) throw new Failure("安装包签名不匹配，已阻止安装");
        if (!certificate(installed.signatures).equals(expectedCertificate))
            throw new Failure("当前安装的是开发签名版本，无法覆盖升级；请先卸载后安装正式版");
        if (Build.VERSION.SDK_INT < 24 && root.equals(activity.getDir("update_staging", Activity.MODE_PRIVATE).getCanonicalFile())) {
            // No shared storage: grant traversal only to this APK-only directory,
            // and read permission to the already verified public release APK.
            // Never change files/, preferences, identities or the app data root.
            if (!root.setExecutable(true, false) || !file.getParentFile().setExecutable(true, false)
                    || !file.setReadable(true, false)) throw new Failure("无法准备旧系统安装文件，请检查存储权限");
        }
        return file;
    }
    private static String certificate(Signature[] signatures) throws Exception {
        if (signatures == null || signatures.length != 1) throw new Failure("无法确认应用签名");
        byte[] digest = MessageDigest.getInstance("SHA-256").digest(signatures[0].toByteArray());
        StringBuilder result = new StringBuilder(64);
        for (byte value : digest) result.append(String.format(java.util.Locale.ROOT, "%02x", value & 255));
        return result.toString();
    }
    static void launch(Activity activity, File file, String relative) {
        if (Build.VERSION.SDK_INT >= 26 && !activity.getPackageManager().canRequestPackageInstalls()) {
            activity.startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:" + activity.getPackageName())));
            return;
        }
        // KitKat's PackageInstaller rejects content://. App-specific external files
        // need no storage permission from API 19; only the public signed APK goes there.
        Uri uri = Build.VERSION.SDK_INT < 24 ? Uri.fromFile(file)
                : Uri.parse("content://" + activity.getPackageName() + ".updates/" + relative);
        Intent intent = new Intent(Intent.ACTION_VIEW).setDataAndType(uri, UpdateProvider.MIME);
        if (Build.VERSION.SDK_INT >= 24) {
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            intent.setClipData(ClipData.newRawUri("ATV update", uri));
        }
        activity.startActivity(intent);
    }
}
