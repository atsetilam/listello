//! Watching folders for "is there anything in it?", for the folders widget.
//!
//! The renderer sends a `key` and a `path` once ([`CoreToAgent::WatchFolder`]); the agent
//! watches the directory with inotify and answers with the key and a yes/no. The path never
//! goes back (D11). Only changes are reported: a folder that gains its tenth file is still
//! not empty, and nobody needs telling.

use listello_protocol::FolderState;
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, WatchDescriptor};
use std::fs;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};

const EVENTS: AddWatchFlags = AddWatchFlags::IN_CREATE
    .union(AddWatchFlags::IN_DELETE)
    .union(AddWatchFlags::IN_MOVED_FROM)
    .union(AddWatchFlags::IN_MOVED_TO)
    .union(AddWatchFlags::IN_DELETE_SELF)
    .union(AddWatchFlags::IN_MOVE_SELF);

struct Watched {
    key: String,
    dir: PathBuf,
    wd: WatchDescriptor,
    /// What was last reported, so an unchanged answer is never sent twice.
    reported: Option<bool>,
}

pub struct Folders {
    inotify: Inotify,
    watched: Vec<Watched>,
    /// Where `trash:/` lives: `$XDG_DATA_HOME/Trash/files`.
    trash: Option<PathBuf>,
    home: Option<PathBuf>,
}

impl Folders {
    pub fn new(home: Option<PathBuf>, data_home: Option<PathBuf>) -> Option<Folders> {
        let inotify = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).ok()?;
        let trash = data_home
            .or_else(|| home.as_ref().map(|h| h.join(".local/share")))
            .map(|d| d.join("Trash/files"));
        Some(Folders {
            inotify,
            watched: Vec::new(),
            trash,
            home,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.inotify.as_fd()
    }

    /// Start watching `path` under `key`, and return its current state. `None` for a path
    /// that is not a plain absolute directory path (or `~/…`, or `trash:/`): the config is
    /// the user's, but it is still not trusted to name anything unexpected.
    pub fn watch(&mut self, key: String, path: &str) -> Option<FolderState> {
        let dir = self.resolve(path)?;
        // Re-sending a watch for a key replaces it, so a config reload does not stack them.
        self.watched.retain(|w| w.key != key);
        // A folder that does not exist yet (an empty Trash is created lazily) cannot be
        // watched, but it is empty: report that and let the next reload pick it up.
        let wd = self.inotify.add_watch(&dir, EVENTS).ok();
        let empty = is_empty(&dir);
        if let Some(wd) = wd {
            self.watched.push(Watched {
                key: key.clone(),
                dir,
                wd,
                reported: Some(empty),
            });
        }
        Some(FolderState { key, empty })
    }

    /// Drain inotify and return the folders whose answer changed.
    pub fn dispatch(&mut self) -> Vec<FolderState> {
        let mut touched = Vec::new();
        while let Ok(events) = self.inotify.read_events() {
            if events.is_empty() {
                break;
            }
            touched.extend(events.into_iter().map(|e| e.wd));
        }
        let mut changed = Vec::new();
        for watched in &mut self.watched {
            if !touched.contains(&watched.wd) {
                continue;
            }
            let empty = is_empty(&watched.dir);
            if watched.reported != Some(empty) {
                watched.reported = Some(empty);
                changed.push(FolderState {
                    key: watched.key.clone(),
                    empty,
                });
            }
        }
        changed
    }

    /// Every watched folder's state, for the greeting a reconnecting renderer needs.
    pub fn all(&mut self) -> Vec<FolderState> {
        self.watched
            .iter_mut()
            .map(|w| {
                let empty = is_empty(&w.dir);
                w.reported = Some(empty);
                FolderState {
                    key: w.key.clone(),
                    empty,
                }
            })
            .collect()
    }

    fn resolve(&self, path: &str) -> Option<PathBuf> {
        if path == "trash:/" {
            return self.trash.clone();
        }
        let resolved = match path.strip_prefix("~/") {
            Some(rest) => self.home.as_ref()?.join(rest),
            None => PathBuf::from(path),
        };
        resolved.is_absolute().then_some(resolved)
    }
}

fn is_empty(dir: &Path) -> bool {
    fs::read_dir(dir).map_or(true, |mut entries| entries.next().is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::fixture::TempDir;

    fn folders(home: &TempDir) -> Folders {
        Folders::new(Some(home.path().to_path_buf()), None).expect("inotify")
    }

    /// inotify is asynchronous by a hair; wait for it to have something instead of sleeping a
    /// guessed amount.
    fn dispatch_soon(folders: &mut Folders) -> Vec<FolderState> {
        for _ in 0..100 {
            let changed = folders.dispatch();
            if !changed.is_empty() {
                return changed;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Vec::new()
    }

    #[test]
    fn watching_reports_the_current_state_and_then_only_changes() {
        let home = TempDir::new("home");
        home.write("Downloads/old.txt", "x");
        let mut folders = folders(&home);
        let first = folders.watch("down".into(), "~/Downloads").expect("state");
        assert_eq!(
            first,
            FolderState {
                key: "down".into(),
                empty: false
            }
        );

        // Another file in a non-empty folder: nothing to report.
        home.write("Downloads/second.txt", "x");
        assert!(dispatch_soon(&mut folders).is_empty());

        fs::remove_file(home.path().join("Downloads/old.txt")).unwrap();
        fs::remove_file(home.path().join("Downloads/second.txt")).unwrap();
        assert_eq!(
            dispatch_soon(&mut folders),
            vec![FolderState {
                key: "down".into(),
                empty: true
            }]
        );

        home.write("Downloads/new.txt", "x");
        assert_eq!(
            dispatch_soon(&mut folders),
            vec![FolderState {
                key: "down".into(),
                empty: false
            }]
        );
    }

    #[test]
    fn trash_is_the_xdg_trash_files_directory() {
        let home = TempDir::new("trash");
        home.write(".local/share/Trash/files/doomed.txt", "x");
        let mut folders = folders(&home);
        let state = folders.watch("trash".into(), "trash:/").expect("state");
        assert!(!state.empty);
        fs::remove_file(home.path().join(".local/share/Trash/files/doomed.txt")).unwrap();
        assert_eq!(
            dispatch_soon(&mut folders),
            vec![FolderState {
                key: "trash".into(),
                empty: true
            }]
        );
    }

    #[test]
    fn a_folder_that_does_not_exist_yet_is_empty() {
        let home = TempDir::new("missing");
        let mut folders = folders(&home);
        let state = folders
            .watch("shots".into(), "~/Screenshots")
            .expect("state");
        assert!(state.empty);
    }

    #[test]
    fn paths_that_are_not_absolute_are_refused() {
        let home = TempDir::new("refuse");
        let mut folders = folders(&home);
        assert_eq!(folders.watch("a".into(), "relative/dir"), None);
        assert_eq!(folders.watch("b".into(), ""), None);
    }

    #[test]
    fn watching_a_key_again_replaces_the_old_watch() {
        let home = TempDir::new("again");
        home.write("A/file", "x");
        let mut folders = folders(&home);
        folders.watch("k".into(), "~/A").expect("first");
        folders.watch("k".into(), "~/A").expect("second");
        assert_eq!(folders.all().len(), 1);
    }
}
