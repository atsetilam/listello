//! Things the agent does to the user's session on the renderer's behalf: start an app, open a
//! folder, lock the screen.
//!
//! Each is a child process started detached from the agent's own terminal and session, so
//! closing or restarting the agent never kills what it launched. The children are kept only
//! to be reaped (no threads, no signal handlers: D3), by [`Spawner::reap`] on each wake.
//!
//! Detaching from the session is not enough under systemd. The agent runs as a user service,
//! and everything it starts lands in that service's cgroup, which systemd kills whole when the
//! agent stops or restarts. Chrome moves its main process into a scope of its own but leaves
//! its helpers behind (the `cat`s its wrapper pipes output through, its renderers), and when
//! those are killed the main process dies on a failed check: a reinstall of listello crashed
//! the user's browser. So apps and file managers start in a scope of their own, the way a
//! desktop launcher starts them.

use crate::desktop::{exec_argv, Catalog};
use listello_protocol::AppId;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};

/// The agent blocks the signals it reads through a signalfd (`reload`), and a blocked mask
/// survives `exec`: an app started from the Dock would never see them. Clear it in the child.
pub fn unblock_signals(command: &mut Command) {
    // SAFETY: only `pthread_sigmask` runs between fork and exec, which is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            nix::sys::signal::SigSet::empty()
                .thread_set_mask()
                .map_err(std::io::Error::from)
        });
    }
}

#[derive(Default)]
pub struct Spawner {
    children: Vec<Child>,
}

impl Spawner {
    /// Start `argv` detached, in a scope named after `scope` when one is given and the agent
    /// runs under systemd. Reports failure on stderr; a press that cannot launch anything is
    /// not worth more than a log line.
    fn spawn(&mut self, argv: &[String], scope: Option<&str>) -> bool {
        if argv.is_empty() {
            return false;
        }
        let argv = match scope {
            Some(name) if under_systemd() => in_own_scope(name, argv),
            _ => argv.to_vec(),
        };
        let (program, args) = argv.split_first().expect("not empty");
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Its own process group, so a signal to the agent's group does not reach it.
            .process_group(0);
        unblock_signals(&mut command);
        let child = command.spawn();
        match child {
            Ok(child) => {
                self.children.push(child);
                true
            }
            Err(e) => {
                eprintln!("listello-agent: could not start {program}: {e}");
                false
            }
        }
    }

    /// Collect children that have exited.
    pub fn reap(&mut self) {
        self.children
            .retain_mut(|c| matches!(c.try_wait(), Ok(None)));
    }

    /// Launch an app that is not running. `false` when no desktop entry says how.
    pub fn launch(&mut self, catalog: &Catalog, app_id: &AppId) -> bool {
        let Some(entry) = catalog.find(app_id.as_str()) else {
            eprintln!(
                "listello-agent: no desktop entry for {:?}, cannot launch it",
                app_id.as_str()
            );
            return false;
        };
        let Some(exec) = entry.exec.as_deref() else {
            return false;
        };
        let mut argv = exec_argv(exec);
        if entry.terminal {
            // The spec leaves the terminal to the desktop; `xdg-terminal-exec` is the
            // closest thing to a standard.
            argv.insert(0, "xdg-terminal-exec".to_string());
        }
        self.spawn(&argv, Some(app_id.as_str()))
    }

    /// Open a path in the user's file manager.
    pub fn open_path(&mut self, path: &str) -> bool {
        self.spawn(
            &["xdg-open".to_string(), expand_home(path)],
            Some("xdg-open"),
        )
    }

    pub fn lock_session(&mut self) -> bool {
        // Done in a moment, so nothing to lose to a restart.
        self.spawn(&["loginctl".to_string(), "lock-session".to_string()], None)
    }
}

/// Whether the agent is a systemd unit (systemd sets `INVOCATION_ID` for every unit it starts)
/// and `systemd-run` is there to start a scope with. Run by hand from a terminal, it is
/// neither, and a scope would buy nothing.
fn under_systemd() -> bool {
    std::env::var_os("INVOCATION_ID").is_some()
        && std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| dir.join("systemd-run").is_file())
        })
}

/// `argv`, to be started in a transient scope of its own under the user's `app.slice`, named
/// `app-listello-<name>-<random>.scope` after the freedesktop convention for launched apps.
fn in_own_scope(name: &str, argv: &[String]) -> Vec<String> {
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let random = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos())
        ^ std::process::id();
    let mut out = vec![
        "systemd-run".to_string(),
        "--user".to_string(),
        "--scope".to_string(),
        "--quiet".to_string(),
        "--collect".to_string(),
        "--slice=app.slice".to_string(),
        format!("--unit=app-listello-{name}-{random:08x}"),
        "--".to_string(),
    ];
    out.extend(argv.iter().cloned());
    out
}

/// `~` and `~/x` mean the user's home; the config file is where the user writes them, and the
/// renderer (as `nobody`) cannot expand them, so it is done here.
pub fn expand_home(path: &str) -> String {
    let home = crate::desktop::home();
    match (path, home) {
        ("~", Some(home)) => home.to_string_lossy().into_owned(),
        (p, Some(home)) if p.starts_with("~/") => home.join(&p[2..]).to_string_lossy().into_owned(),
        (p, _) => p.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tilde_path_is_expanded_and_others_are_left_alone() {
        let home = crate::desktop::home().expect("HOME is set in tests");
        assert_eq!(expand_home("~"), home.to_string_lossy());
        assert_eq!(
            expand_home("~/Downloads"),
            home.join("Downloads").to_string_lossy()
        );
        assert_eq!(expand_home("/tmp/x"), "/tmp/x");
        assert_eq!(expand_home("~other/x"), "~other/x");
    }

    #[test]
    fn an_app_is_started_in_a_scope_of_its_own_named_after_it() {
        let argv = in_own_scope(
            "com.google.Chrome/x y",
            &["chrome".into(), "--new-window".into()],
        );
        assert_eq!(
            &argv[..6],
            [
                "systemd-run",
                "--user",
                "--scope",
                "--quiet",
                "--collect",
                "--slice=app.slice"
            ]
        );
        assert!(
            argv[6].starts_with("--unit=app-listello-com.google.Chrome_x_y-"),
            "{}",
            argv[6]
        );
        assert_eq!(&argv[7..], ["--", "chrome", "--new-window"]);
    }

    #[test]
    fn an_empty_command_is_refused() {
        assert!(!Spawner::default().spawn(&[], None));
        assert!(!Spawner::default().spawn(&[], Some("app")));
    }

    #[test]
    fn a_finished_child_is_reaped() {
        let mut spawner = Spawner::default();
        assert!(spawner.spawn(&["true".to_string()], None));
        for _ in 0..200 {
            spawner.reap();
            if spawner.children.is_empty() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the child was never reaped");
    }
}
