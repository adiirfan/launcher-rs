package dev.launcherrs.app;

import android.service.notification.NotificationListenerService;
import android.service.notification.StatusBarNotification;
import java.util.LinkedHashSet;

/** Tells Rust which apps have notifications, for the dots on their icons.
 *  Android starts it once the owner allows notification access; that same
 *  access lets the launcher see what music is playing. */
public class DotsListener extends NotificationListenerService {
    static { System.loadLibrary("launcher"); }

    @Override public void onListenerConnected() { report(); }
    @Override public void onListenerDisconnected() { onDots(""); }
    @Override public void onNotificationPosted(StatusBarNotification n) { report(); }
    @Override public void onNotificationRemoved(StatusBarNotification n) { report(); }
    @Override public void onNotificationRankingUpdate(RankingMap r) { report(); }

    /** Apps with a notification the owner can swipe away, whose channel allows a dot. */
    private void report() {
        StatusBarNotification[] all;
        RankingMap ranks;
        try {
            all = getActiveNotifications();
            ranks = getCurrentRanking();
        } catch (RuntimeException e) {
            return;
        }
        if (all == null) return;
        Ranking r = new Ranking();
        LinkedHashSet<String> apps = new LinkedHashSet<>();
        for (StatusBarNotification n : all) {
            if (!n.isClearable() || n.getPackageName().equals(getPackageName())) continue;
            if (ranks != null && ranks.getRanking(n.getKey(), r) && !r.canShowBadge()) continue;
            apps.add(n.getPackageName());
        }
        onDots(String.join("\n", apps));
    }

    private static native void onDots(String packages);
}
