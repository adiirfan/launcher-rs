package dev.launcherrs.gallery;

import android.app.Activity;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.PackageInstaller;
import android.net.Uri;
import android.os.Build;
import android.provider.Settings;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;

/**
 * Checks GitHub for a newer release and installs it. Goes online only when
 * the user taps "Check for updates"; never on its own.
 */
final class Updater {
    static final int IDLE = 0, CHECKING = 1, UP_TO_DATE = 2, AVAILABLE = 3,
            DOWNLOADING = 4, INSTALLING = 5, FAILED = 6, NEEDS_PERMISSION = 7;

    /** Newest releases first. The launcher's releases live in the same repo,
     *  so we pick the newest "gallery-v…" one ourselves. */
    private static final String RELEASES =
            "https://api.github.com/repos/adiirfan/launcher-rs/releases?per_page=30";
    private static final String STATUS = "dev.launcherrs.gallery.INSTALL_STATUS";
    private static final int TIMEOUT_MS = 15000;

    /** The newest release's app file, once a check found one. */
    private static volatile String apkUrl;
    private static volatile boolean busy;

    /** State changed: what to show (a version or a reason) and progress. */
    static native void onUpdate(int state, String text, int percent);

    private Updater() {}

    static String myVersion(Context c) {
        try {
            return c.getPackageManager().getPackageInfo(c.getPackageName(), 0).versionName;
        } catch (Exception e) {
            return "?";
        }
    }

    static void check(Activity a) {
        if (busy) return;
        busy = true;
        onUpdate(CHECKING, "", 0);
        new Thread(() -> {
            try {
                HttpURLConnection c = open(RELEASES);
                c.setRequestProperty("Accept", "application/vnd.github+json");
                int code = c.getResponseCode();
                if (code != 200) {
                    onUpdate(FAILED, "GitHub answered " + code, 0);
                    return;
                }
                JSONArray releases = new JSONArray(readAll(c.getInputStream()));
                String latest = null, url = null;
                for (int r = 0; r < releases.length() && latest == null; r++) {
                    JSONObject release = releases.getJSONObject(r);
                    if (release.optBoolean("draft") || release.optBoolean("prerelease")) continue;
                    String version = galleryVersion(release.getString("tag_name"));
                    if (version == null) continue;
                    JSONArray assets = release.getJSONArray("assets");
                    for (int i = 0; i < assets.length(); i++) {
                        JSONObject asset = assets.getJSONObject(i);
                        if (asset.getString("name").endsWith(".apk")) {
                            url = asset.getString("browser_download_url");
                        }
                    }
                    if (url != null) latest = version;
                }
                String mine = myVersion(a);
                if (latest != null && newer(latest, mine)) {
                    apkUrl = url;
                    onUpdate(AVAILABLE, latest, 0);
                } else {
                    onUpdate(UP_TO_DATE, mine, 0);
                }
            } catch (Exception e) {
                onUpdate(FAILED, "No connection", 0);
            } finally {
                busy = false;
            }
        }, "update-check").start();
    }

    static void install(Activity a) {
        String url = apkUrl;
        if (busy || url == null) return;
        // Android asks once whether the gallery may install apps.
        if (Build.VERSION.SDK_INT >= 26 && !a.getPackageManager().canRequestPackageInstalls()) {
            onUpdate(NEEDS_PERMISSION, "", 0);
            a.startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:" + a.getPackageName())));
            return;
        }
        busy = true;
        onUpdate(DOWNLOADING, "", 0);
        new Thread(() -> {
            PackageInstaller.Session session = null;
            try {
                HttpURLConnection c = open(url);
                if (c.getResponseCode() != 200) {
                    onUpdate(FAILED, "Download refused (" + c.getResponseCode() + ")", 0);
                    return;
                }
                long size = c.getContentLengthLong();
                PackageInstaller pi = a.getPackageManager().getPackageInstaller();
                PackageInstaller.SessionParams params =
                        new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
                params.setAppPackageName(a.getPackageName());
                if (size > 0) params.setSize(size);
                session = pi.openSession(pi.createSession(params));
                try (InputStream in = c.getInputStream();
                     OutputStream out = session.openWrite("gallery.apk", 0, size)) {
                    byte[] buf = new byte[64 * 1024];
                    long done = 0;
                    int lastPct = -1, n;
                    while ((n = in.read(buf)) > 0) {
                        out.write(buf, 0, n);
                        done += n;
                        int pct = size > 0 ? (int) (done * 100 / size) : 0;
                        if (pct != lastPct) {
                            lastPct = pct;
                            onUpdate(DOWNLOADING, "", pct);
                        }
                    }
                    session.fsync(out);
                }
                listenForResult(a);
                Intent status = new Intent(STATUS).setPackage(a.getPackageName());
                int flags = PendingIntent.FLAG_UPDATE_CURRENT
                        | (Build.VERSION.SDK_INT >= 31 ? PendingIntent.FLAG_MUTABLE : 0);
                session.commit(PendingIntent.getBroadcast(a, 0, status, flags).getIntentSender());
                session.close();
                session = null;
                onUpdate(INSTALLING, "", 100);
            } catch (Exception e) {
                if (session != null) session.abandon();
                onUpdate(FAILED, "Download failed", 0);
            } finally {
                busy = false;
            }
        }, "update-install").start();
    }

    /** Android reports back here: show its "Update this app?" prompt, or why it failed. */
    private static void listenForResult(Activity a) {
        BroadcastReceiver r = new BroadcastReceiver() {
            @Override public void onReceive(Context ctx, Intent intent) {
                int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
                if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
                    Intent confirm = intent.getParcelableExtra(Intent.EXTRA_INTENT);
                    if (confirm != null) {
                        confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                        ctx.startActivity(confirm);
                    }
                    return;
                }
                try { ctx.unregisterReceiver(this); } catch (Exception ignored) {}
                if (status != PackageInstaller.STATUS_SUCCESS) {
                    onUpdate(FAILED, status == PackageInstaller.STATUS_FAILURE_ABORTED
                            ? "Update cancelled" : "Android refused the update", 0);
                }
            }
        };
        IntentFilter filter = new IntentFilter(STATUS);
        if (Build.VERSION.SDK_INT >= 33) {
            a.registerReceiver(r, filter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            a.registerReceiver(r, filter);
        }
    }

    private static HttpURLConnection open(String url) throws Exception {
        HttpURLConnection c = (HttpURLConnection) new URL(url).openConnection();
        c.setConnectTimeout(TIMEOUT_MS);
        c.setReadTimeout(TIMEOUT_MS);
        c.setInstanceFollowRedirects(true);
        c.setRequestProperty("User-Agent", "launcher-rs-gallery-updater");
        return c;
    }

    private static String readAll(InputStream in) throws Exception {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buf = new byte[8192];
        int n;
        while ((n = in.read(buf)) > 0) out.write(buf, 0, n);
        in.close();
        return out.toString("UTF-8");
    }

    /** "gallery-v0.1.0" → "0.1.0"; anything else (a launcher release) → null. */
    static String galleryVersion(String tag) {
        String v = tag.startsWith("gallery-v") ? tag.substring(9) : null;
        return v != null && !v.isEmpty() && Character.isDigit(v.charAt(0)) ? v : null;
    }

    /** Whether version `a` (like "0.1.10") is newer than `b`. */
    static boolean newer(String a, String b) {
        String[] x = a.split("\\."), y = b.split("\\.");
        for (int i = 0; i < Math.max(x.length, y.length); i++) {
            int p = i < x.length ? num(x[i]) : 0, q = i < y.length ? num(y[i]) : 0;
            if (p != q) return p > q;
        }
        return false;
    }

    private static int num(String s) {
        try { return Integer.parseInt(s.replaceAll("[^0-9].*$", "")); } catch (Exception e) { return 0; }
    }
}
