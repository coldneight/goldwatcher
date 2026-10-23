//! Native desktop notifications.

use std::thread;

/// Send a desktop notification without blocking the UI thread.
pub fn notify(title: String, body: String, silent: bool) {
    thread::spawn(move || {
        let mut notification = notify_rust::Notification::new();
        notification
            .summary(&title)
            .body(&body)
            .appname("GoldWatcher")
            .icon("gold-watcher")
            .timeout(notify_rust::Timeout::Milliseconds(if silent {
                4000
            } else {
                9000
            }));
        if !silent {
            let _ = notification.sound_name("message-new-instant");
        }
        if let Err(err) = notification.show() {
            eprintln!("GoldWatcher: 桌面通知发送失败：{err}");
        }
    });
}
