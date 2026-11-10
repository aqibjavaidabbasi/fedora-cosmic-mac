//! Event-driven updates for the cosmic-mac applets, so an idle top bar does no work.
//! Shared by the cosmic-mac applets (the same file is copied into each crate).
//!
//! In COSMIC's toolkit every message an applet receives rebuilds and redraws it (and the
//! panel composites it again), so nothing here sends a message unless something changed:
//!  - `open_requests`: inotify on `$XDG_RUNTIME_DIR/cosmic-mac/open` instead of a timer.
//!  - `file`: inotify on one file, yielding only when the part we show changes.
//!  - `changes`: a sampler on a timer that yields only when its result differs.

#![allow(dead_code)]

use std::{ffi::OsStr, future::Future, path::PathBuf, time::Duration};

use cosmic::iced::{
    Subscription,
    futures::{SinkExt, StreamExt, Stream, channel::mpsc::Sender},
    stream,
};
use inotify::{EventMask, Inotify, WatchMask};

fn runtime_dir() -> PathBuf {
    PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into())).join("cosmic-mac")
}

/// Waits for `name` (or any file when `None`) in `dir` to be written and closed, or renamed
/// into place. Returns the name, or `None` when the folder went away (watch it again).
async fn next_write(events: &mut inotify::EventStream<[u8; 1024]>, name: Option<&OsStr>) -> Option<std::ffi::OsString> {
    while let Some(Ok(e)) = events.next().await {
        if e.mask.intersects(EventMask::IGNORED | EventMask::DELETE_SELF) {
            return None;
        }
        if let Some(n) = e.name {
            if name.is_none_or(|w| w == n) {
                return Some(n);
            }
        }
    }
    None
}

fn watch_dir(dir: &PathBuf) -> Option<inotify::EventStream<[u8; 1024]>> {
    let _ = std::fs::create_dir_all(dir);
    let inotify = Inotify::init().ok()?;
    inotify.watches().add(dir, WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::DELETE_SELF).ok()?;
    inotify.into_event_stream([0u8; 1024]).ok()
}

/// Requests to open a popover from scripts and keyboard shortcuts:
///   mkdir -p $XDG_RUNTIME_DIR/cosmic-mac/open && touch $XDG_RUNTIME_DIR/cosmic-mac/open/<app id>
/// Yields the file's text (the Control Center reads a page name from it); the file is removed.
pub fn open_requests(app_id: &'static str) -> Subscription<String> {
    Subscription::run_with(app_id, |id| {
        let id: &'static str = *id;
        stream::channel(4, move |mut tx: Sender<String>| async move {
            let dir = runtime_dir().join("open");
            let file = dir.join(id);
            loop {
                let Some(mut events) = watch_dir(&dir) else {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    continue;
                };
                // a request made before the watch started
                loop {
                    if let Ok(text) = std::fs::read_to_string(&file) {
                        let _ = std::fs::remove_file(&file);
                        if tx.send(text).await.is_err() {
                            return;
                        }
                    }
                    if next_write(&mut events, Some(OsStr::new(id))).await.is_none() {
                        break;
                    }
                }
            }
        })
    })
}

/// Watches one file and yields `key(contents)` at start and whenever that value changes,
/// e.g. only the app id line of a "app id\ntitle" file.
pub fn file(path: String, key: fn(&str) -> String) -> Subscription<String> {
    Subscription::run_with((path, key), |(path, key)| {
        let (path, key) = (PathBuf::from(path), *key);
        stream::channel(4, move |mut tx: Sender<String>| async move {
            let (Some(dir), Some(name)) = (path.parent().map(PathBuf::from), path.file_name().map(|n| n.to_owned())) else {
                return;
            };
            let mut last: Option<String> = None;
            loop {
                let Some(mut events) = watch_dir(&dir) else {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    continue;
                };
                loop {
                    let now = key(&std::fs::read_to_string(&path).unwrap_or_default());
                    if last.as_ref() != Some(&now) {
                        last = Some(now.clone());
                        if tx.send(now).await.is_err() {
                            return;
                        }
                    }
                    if next_write(&mut events, Some(&name)).await.is_none() {
                        break;
                    }
                }
            }
        })
    })
}

/// Runs `sample` (given the last value it yielded) right away and then every `period`,
/// and yields its result only when it differs from the last one.
pub fn changes<T, F, Fut>(period: Duration, mut sample: F) -> impl Stream<Item = T>
where
    T: PartialEq + Clone + Send + 'static,
    F: FnMut(Option<T>) -> Fut + Send + 'static,
    Fut: Future<Output = T> + Send,
{
    stream::channel(1, move |mut tx: Sender<T>| async move {
        let mut last: Option<T> = None;
        loop {
            let now = sample(last.clone()).await;
            if last.as_ref() != Some(&now) {
                last = Some(now.clone());
                if tx.send(now).await.is_err() {
                    return;
                }
            }
            tokio::time::sleep(period).await;
        }
    })
}

/// True while the laptop runs on battery (no mains adapter online).
pub fn on_battery() -> bool {
    let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") else { return false };
    let mut saw_mains = false;
    for e in dir.flatten() {
        let p = e.path();
        if std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Mains") {
            saw_mains = true;
            if std::fs::read_to_string(p.join("online")).is_ok_and(|o| o.trim() == "1") {
                return false;
            }
        }
    }
    saw_mains
}
