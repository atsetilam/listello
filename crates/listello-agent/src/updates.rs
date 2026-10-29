//! Looking for a newer release (D9 amendment, 2026-10-29).
//!
//! The renderer runs as `nobody` and does no networking, so the check lives here, in the user's
//! session. About once a day (a few minutes after the agent starts, then every 24 hours) the
//! agent asks GitHub for the latest release. When it is newer than this build it shows a
//! desktop notification with an action that opens the release page in the browser; listello
//! never downloads or installs anything. Each newer version is announced once by itself, so it
//! does not nag. The tray's "Check for updates" (`SIGHUP`, see `reload`) looks at once and
//! always answers: newer, up to date, or could not check.
//!
//! The request, the notification and the wait for a click run on a short-lived thread of their
//! own, so the single event loop never waits on the network. The thread shares nothing with it:
//! it works from copies and leaves its marks in files under `$XDG_STATE_HOME/listello/`.

use crate::actions::Spawner;
use dbus::arg::{PropMap, RefArg, Variant};
use dbus::blocking::Connection;
use dbus::message::MatchRule;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const LATEST: &str = "https://api.github.com/repos/atsetilam/listello/releases/latest";

/// What this build is.
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// First automatic check after the agent starts: not at login, when the session is busy and
/// the network may not be up yet.
const FIRST_AFTER: Duration = Duration::from_secs(5 * 60);

const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// When the automatic check is off, look at the setting again this often.
const RECHECK_SETTING: Duration = Duration::from_secs(60 * 60);

/// How long a notification's action is listened for. Later clicks (from the notification
/// history) do nothing; the next announcement is a day away at the soonest.
const LISTEN_FOR: Duration = Duration::from_secs(10 * 60);

/// Never more than this many helper threads at once, however often the menu item is clicked.
const MOST_THREADS: usize = 4;

/// Left in the state directory after each attempt (Unix seconds) and each announcement.
const LAST_CHECK: &str = "last-update-check";
const LAST_NOTIFIED: &str = "last-update-notified";

pub type Version = (u64, u64, u64);

/// A published release: what to compare and where to send the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The tag as GitHub names it, such as `v0.1.1`.
    pub tag: String,
    pub url: String,
}

/// `v0.1.1`, `0.2`, `1.0.0-rc1`: up to three numbers; a pre-release or build suffix is ignored.
pub fn parse_version(text: &str) -> Option<Version> {
    let text = text.trim();
    let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
    let core = text.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let mut next = |required: bool| match parts.next() {
        Some(p) => p.parse::<u64>().ok().map(Some),
        None if required => None,
        None => Some(None),
    };
    let major = next(true)??;
    let minor = next(false)?.unwrap_or(0);
    let patch = next(false)?.unwrap_or(0);
    // A fourth component is not a version we know how to compare.
    parts.next().is_none().then_some((major, minor, patch))
}

/// The two fields of GitHub's "latest release" answer that matter. Anything else, or anything
/// malformed, is an error rather than a guess: the page is handed to the browser.
pub fn parse_release(json: &str) -> Result<Release, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("not JSON: {e}"))?;
    let tag = value
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or("no tag_name")?;
    let url = value
        .get("html_url")
        .and_then(|v| v.as_str())
        .ok_or("no html_url")?;
    let version = parse_version(tag).ok_or_else(|| format!("unreadable tag {tag:?}"))?;
    if !url.starts_with("https://") {
        return Err(format!("unexpected release page {url:?}"));
    }
    Ok(Release {
        version,
        tag: tag.to_string(),
        url: url.to_string(),
    })
}

/// Whether to announce `latest` by itself: it is newer than this build and newer than the
/// last version announced (`None`, or an unreadable mark, means none yet).
pub fn should_announce(current: Version, latest: Version, last_notified: Option<&str>) -> bool {
    latest > current
        && last_notified
            .and_then(parse_version)
            .is_none_or(|n| latest > n)
}

/// How long to wait before the next automatic check, given when the last attempt was (Unix
/// seconds). Never less than [`FIRST_AFTER`], so a restart does not look straight away; a mark
/// from the future (the clock moved) counts as no mark.
pub fn wait_after(last_attempt: Option<u64>, now: u64) -> Duration {
    let remaining = last_attempt
        .filter(|&t| t <= now)
        .map(|t| (t + EVERY.as_secs()).saturating_sub(now))
        .map_or(FIRST_AFTER, Duration::from_secs);
    remaining.max(FIRST_AFTER)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Ask GitHub for the latest release. Errors are short, for a log line or a notification.
pub fn fetch_latest() -> Result<Release, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into();
    let mut response = agent
        .get(LATEST)
        .header("User-Agent", &format!("listello/{CURRENT}"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(code @ (403 | 429)) => {
                format!("GitHub is limiting requests (HTTP {code})")
            }
            ureq::Error::StatusCode(code) => format!("GitHub answered HTTP {code}"),
            other => format!("no answer from GitHub: {other}"),
        })?;
    let body = response
        .body_mut()
        .with_config()
        .limit(1 << 20)
        .read_to_string()
        .map_err(|e| format!("could not read the answer: {e}"))?;
    parse_release(&body)
}

// ---- the files the thread leaves behind ---------------------------------------------------

#[derive(Clone, Debug)]
struct Marks {
    dir: PathBuf,
}

impl Marks {
    fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join(name))
            .ok()
            .map(|t| t.trim().to_string())
    }

    fn write(&self, name: &str, text: &str) {
        let done = std::fs::create_dir_all(&self.dir)
            .and_then(|()| std::fs::write(self.dir.join(name), text));
        if let Err(e) = done {
            eprintln!("listello-agent: could not write {name}: {e}");
        }
    }

    fn last_attempt(&self) -> Option<u64> {
        self.read(LAST_CHECK)?.parse().ok()
    }
}

/// `$XDG_STATE_HOME/listello`, else `~/.local/state/listello` (where `asked-start-at-login`
/// lives).
fn state_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| crate::desktop::home().map(|h| h.join(".local/state")))
        .map(|d| d.join("listello"))
}

// ---- the user's file ----------------------------------------------------------------------

/// `[general]` of the user's file, read loosely: these two settings are honoured even while the
/// layout in the same file does not validate.
fn general(config_dir: Option<&Path>) -> Option<toml::Table> {
    let text = std::fs::read_to_string(config_dir?.join("config.toml")).ok()?;
    match text.parse::<toml::Table>().ok()?.remove("general")? {
        toml::Value::Table(t) => Some(t),
        _ => None,
    }
}

fn automatic_wanted(config_dir: Option<&Path>) -> bool {
    general(config_dir)
        .and_then(|g| g.get("check_updates")?.as_bool())
        .unwrap_or(true)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lang {
    English,
    Italian,
}

/// The same choice the settings window makes: `[general] language`, else the display language.
fn language(config_dir: Option<&Path>) -> Lang {
    let setting = general(config_dir)
        .and_then(|g| g.get("language")?.as_str().map(str::to_string))
        .unwrap_or_default();
    match setting.as_str() {
        "italian" => Lang::Italian,
        "english" => Lang::English,
        _ => {
            let display = ["LC_ALL", "LC_MESSAGES", "LANG"]
                .iter()
                .filter_map(|n| std::env::var(n).ok())
                .find(|v| !v.is_empty())
                .unwrap_or_default();
            if display.starts_with("it") {
                Lang::Italian
            } else {
                Lang::English
            }
        }
    }
}

/// What a notification says: (summary, body).
fn words(lang: Lang, what: &Outcome) -> (String, String) {
    match (lang, what) {
        (Lang::English, Outcome::Newer(r)) => (
            format!("listello {} is available", shown(r.version)),
            format!("You have {CURRENT}. Click to see what is new."),
        ),
        (Lang::Italian, Outcome::Newer(r)) => (
            format!("È disponibile listello {}", shown(r.version)),
            format!("Hai la {CURRENT}. Clicca per vedere le novità."),
        ),
        (Lang::English, Outcome::Current) => (
            "listello is up to date".to_string(),
            format!("You have the latest version ({CURRENT})."),
        ),
        (Lang::Italian, Outcome::Current) => (
            "listello è aggiornato".to_string(),
            format!("Hai l'ultima versione ({CURRENT})."),
        ),
        (Lang::English, Outcome::Failed(_)) => (
            "Could not check for updates".to_string(),
            "Are you online? Try again later.".to_string(),
        ),
        (Lang::Italian, Outcome::Failed(_)) => (
            "Impossibile cercare aggiornamenti".to_string(),
            "Sei online? Riprova più tardi.".to_string(),
        ),
    }
}

fn open_label(lang: Lang) -> &'static str {
    match lang {
        Lang::English => "Open release page",
        Lang::Italian => "Apri la pagina della versione",
    }
}

fn shown((a, b, c): Version) -> String {
    format!("{a}.{b}.{c}")
}

// ---- one check ----------------------------------------------------------------------------

enum Outcome {
    Newer(Release),
    Current,
    Failed(String),
}

/// Compare an answer with this build.
fn judge(answer: Result<Release, String>, current: Version) -> Outcome {
    match answer {
        Ok(release) if release.version > current => Outcome::Newer(release),
        Ok(_) => Outcome::Current,
        Err(e) => Outcome::Failed(e),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Quiet unless there is something new that has not been announced.
    Automatic,
    /// Always answers.
    Asked,
}

/// Everything a thread needs, copied out of the main loop.
#[derive(Clone)]
struct Job {
    marks: Option<Marks>,
    config_dir: Option<PathBuf>,
}

impl Job {
    fn run(&self, mode: Mode) {
        let Some(current) = parse_version(CURRENT) else {
            return;
        };
        if mode == Mode::Automatic {
            // Marked before asking, so an unreachable or limiting GitHub is not asked again by
            // every restart.
            if let Some(marks) = &self.marks {
                marks.write(LAST_CHECK, &unix_now().to_string());
            }
        }
        let outcome = judge(fetch_latest(), current);
        let lang = language(self.config_dir.as_deref());
        match (&outcome, mode) {
            (Outcome::Failed(e), _) => eprintln!("listello-agent: update check failed: {e}"),
            (Outcome::Newer(r), _) => eprintln!("listello-agent: {} is available", r.tag),
            (Outcome::Current, _) => eprintln!("listello-agent: up to date"),
        }
        match (&outcome, mode) {
            (Outcome::Newer(release), Mode::Automatic) => {
                let last = self.marks.as_ref().and_then(|m| m.read(LAST_NOTIFIED));
                if should_announce(current, release.version, last.as_deref()) {
                    self.announce(release, lang);
                }
            }
            (Outcome::Newer(release), Mode::Asked) => self.announce(release, lang),
            (_, Mode::Asked) => {
                let (summary, body) = words(lang, &outcome);
                if let Err(e) = notify(&summary, &body, None, lang) {
                    eprintln!("listello-agent: could not show a notification: {e}");
                }
            }
            (_, Mode::Automatic) => {}
        }
    }

    fn announce(&self, release: &Release, lang: Lang) {
        let (summary, body) = words(lang, &Outcome::Newer(release.clone()));
        if let Some(marks) = &self.marks {
            marks.write(LAST_NOTIFIED, &release.tag);
        }
        if let Err(e) = notify(&summary, &body, Some(&release.url), lang) {
            eprintln!("listello-agent: could not show a notification: {e}");
        }
    }
}

/// Show a notification and, when it carries a page, open it if the user clicks. Returns after
/// the notification is closed or [`LISTEN_FOR`] has passed.
fn notify(summary: &str, body: &str, page: Option<&str>, lang: Lang) -> Result<(), String> {
    let conn = Connection::new_session().map_err(|e| format!("no session bus: {e}"))?;
    let invoked = Arc::new(AtomicU32::new(0));
    let closed = Arc::new(AtomicU32::new(0));
    if page.is_some() {
        let seen = invoked.clone();
        conn.add_match(
            MatchRule::new_signal("org.freedesktop.Notifications", "ActionInvoked"),
            move |(id, _action): (u32, String), _, _| {
                seen.store(id, Ordering::SeqCst);
                true
            },
        )
        .map_err(|e| e.to_string())?;
        let seen = closed.clone();
        conn.add_match(
            MatchRule::new_signal("org.freedesktop.Notifications", "NotificationClosed"),
            move |(id, _why): (u32, u32), _, _| {
                seen.store(id, Ordering::SeqCst);
                true
            },
        )
        .map_err(|e| e.to_string())?;
    }

    let mut hints = PropMap::new();
    hints.insert(
        "desktop-entry".to_string(),
        Variant(Box::new("org.listello.Settings".to_string()) as Box<dyn RefArg>),
    );
    let label = open_label(lang);
    let actions: Vec<&str> = if page.is_some() {
        vec!["default", label, "open", label]
    } else {
        Vec::new()
    };
    let proxy = conn.with_proxy(
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        Duration::from_secs(5),
    );
    let (id,): (u32,) = proxy
        .method_call(
            "org.freedesktop.Notifications",
            "Notify",
            (
                "listello", 0u32, "listello", summary, body, actions, hints, -1i32,
            ),
        )
        .map_err(|e| e.to_string())?;

    let Some(page) = page else {
        return Ok(());
    };
    let end = Instant::now() + LISTEN_FOR;
    while Instant::now() < end {
        conn.process(Duration::from_millis(500))
            .map_err(|e| e.to_string())?;
        if invoked.load(Ordering::SeqCst) == id {
            let mut spawner = Spawner::default();
            spawner.open_path(page);
            spawner.finish(Duration::from_secs(30));
            return Ok(());
        }
        if closed.load(Ordering::SeqCst) == id {
            return Ok(());
        }
    }
    Ok(())
}

// ---- the schedule -------------------------------------------------------------------------

pub struct Updates {
    job: Job,
    next: Instant,
    threads: Vec<JoinHandle<()>>,
}

impl Updates {
    /// `config_dir` is `~/.config/listello`, where the user's file is.
    pub fn new(config_dir: Option<PathBuf>, now: Instant) -> Updates {
        let marks = state_dir().map(|dir| Marks { dir });
        let wait = wait_after(marks.as_ref().and_then(Marks::last_attempt), unix_now());
        Updates {
            job: Job { marks, config_dir },
            next: now + wait,
            threads: Vec::new(),
        }
    }

    /// How long until the next automatic check is due.
    pub fn until_due(&self, now: Instant) -> Duration {
        self.next.saturating_duration_since(now)
    }

    /// Start the automatic check if it is due and wanted.
    pub fn tick(&mut self, now: Instant) {
        self.threads.retain(|t| !t.is_finished());
        if now < self.next {
            return;
        }
        if !automatic_wanted(self.job.config_dir.as_deref()) {
            self.next = now + RECHECK_SETTING;
            return;
        }
        self.next = now + EVERY;
        self.start(Mode::Automatic);
    }

    /// "Check for updates" from the tray: now, whatever the setting, and with an answer.
    pub fn check_now(&mut self) {
        self.threads.retain(|t| !t.is_finished());
        self.start(Mode::Asked);
    }

    fn start(&mut self, mode: Mode) {
        if self.threads.len() >= MOST_THREADS {
            return;
        }
        let job = self.job.clone();
        match std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || job.run(mode))
        {
            Ok(handle) => self.threads.push(handle),
            Err(e) => eprintln!("listello-agent: could not start the update check: {e}"),
        }
    }
}

/// `--check-updates`: look once, print the answer, and show nothing.
pub fn print_check() {
    let Some(current) = parse_version(CURRENT) else {
        return;
    };
    match judge(fetch_latest(), current) {
        Outcome::Newer(r) => println!("listello {} is available: {}", r.tag, r.url),
        Outcome::Current => println!("listello {CURRENT} is up to date"),
        Outcome::Failed(e) => {
            eprintln!("could not check: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::fixture::TempDir;

    #[test]
    fn versions_read_with_or_without_a_v_and_ignore_suffixes() {
        assert_eq!(parse_version("v0.1.1"), Some((0, 1, 1)));
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("V2.0"), Some((2, 0, 0)));
        assert_eq!(parse_version("3"), Some((3, 0, 0)));
        assert_eq!(parse_version("v1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3+build5"), Some((1, 2, 3)));
        assert_eq!(parse_version(" v0.1.0\n"), Some((0, 1, 0)));
    }

    #[test]
    fn things_that_are_not_versions_are_refused() {
        for bad in [
            "", "v", "latest", "1.x.3", "1..2", "1.2.3.4", "-1.0.0", "v1.2.-3",
        ] {
            assert_eq!(parse_version(bad), None, "{bad:?}");
        }
        assert_eq!(parse_version("99999999999999999999.0.0"), None);
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        let v = |s| parse_version(s).unwrap();
        assert!(v("0.1.10") > v("0.1.9"));
        assert!(v("0.2.0") > v("0.1.99"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("0.1.0") == v("v0.1.0"));
    }

    const GOOD: &str = r#"{"tag_name":"v0.1.1","name":"0.1.1","draft":false,
        "html_url":"https://github.com/atsetilam/listello/releases/tag/v0.1.1","assets":[]}"#;

    #[test]
    fn a_release_answer_gives_its_tag_and_page() {
        let r = parse_release(GOOD).unwrap();
        assert_eq!(r.version, (0, 1, 1));
        assert_eq!(r.tag, "v0.1.1");
        assert_eq!(
            r.url,
            "https://github.com/atsetilam/listello/releases/tag/v0.1.1"
        );
    }

    #[test]
    fn a_bad_answer_is_an_error_not_a_panic() {
        for bad in [
            "",
            "<html>rate limited</html>",
            "[]",
            "null",
            r#"{"message":"API rate limit exceeded"}"#,
            r#"{"tag_name":"v0.1.1"}"#,
            r#"{"html_url":"https://x.org/"}"#,
            r#"{"tag_name":7,"html_url":"https://x.org/"}"#,
            r#"{"tag_name":"nightly","html_url":"https://x.org/"}"#,
            r#"{"tag_name":"v0.1.1","html_url":"file:///etc/passwd"}"#,
            r#"{"tag_name":"v0.1.1","html_url":"http://x.org/"}"#,
            r#"{"tag_name":"v0.1.1","html_url":"#,
        ] {
            assert!(parse_release(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_version_is_announced_once() {
        let current = (0, 1, 0);
        // Newer and never announced.
        assert!(should_announce(current, (0, 1, 1), None));
        // Announced already.
        assert!(!should_announce(current, (0, 1, 1), Some("v0.1.1")));
        // A still newer one is announced again.
        assert!(should_announce(current, (0, 1, 2), Some("v0.1.1")));
        // Not newer than this build: never.
        assert!(!should_announce(current, (0, 1, 0), None));
        assert!(!should_announce(current, (0, 0, 9), None));
        // A mark nobody can read counts as no mark.
        assert!(should_announce(current, (0, 1, 1), Some("garbage")));
    }

    #[test]
    fn a_newer_release_is_judged_against_this_build() {
        let release = parse_release(GOOD).unwrap();
        assert!(matches!(
            judge(Ok(release.clone()), (0, 1, 0)),
            Outcome::Newer(_)
        ));
        assert!(matches!(
            judge(Ok(release.clone()), (0, 1, 1)),
            Outcome::Current
        ));
        assert!(matches!(judge(Ok(release), (0, 2, 0)), Outcome::Current));
        assert!(matches!(
            judge(Err("offline".into()), (0, 1, 0)),
            Outcome::Failed(_)
        ));
    }

    #[test]
    fn the_first_check_waits_a_few_minutes_and_the_rest_a_day_from_the_last() {
        let day = EVERY.as_secs();
        let now = 1_000_000;
        assert_eq!(wait_after(None, now), FIRST_AFTER);
        // Just checked: a day, less what has passed.
        assert_eq!(
            wait_after(Some(now - 3600), now),
            Duration::from_secs(day - 3600)
        );
        // Overdue: still not at once.
        assert_eq!(wait_after(Some(now - 2 * day), now), FIRST_AFTER);
        // Nearly due: not sooner than the first delay.
        assert_eq!(wait_after(Some(now - day + 5), now), FIRST_AFTER);
        // A mark from the future is no mark.
        assert_eq!(wait_after(Some(now + 500), now), FIRST_AFTER);
    }

    #[test]
    fn the_marks_survive_between_runs() {
        let root = TempDir::new("update-marks");
        let marks = Marks {
            dir: root.path().join("listello"),
        };
        assert_eq!(marks.last_attempt(), None);
        assert_eq!(marks.read(LAST_NOTIFIED), None);
        marks.write(LAST_CHECK, "1234");
        marks.write(LAST_NOTIFIED, "v0.1.1");
        assert_eq!(marks.last_attempt(), Some(1234));
        assert_eq!(marks.read(LAST_NOTIFIED).as_deref(), Some("v0.1.1"));
        marks.write(LAST_CHECK, "not a number");
        assert_eq!(marks.last_attempt(), None);
    }

    #[test]
    fn the_automatic_check_is_on_unless_the_file_turns_it_off() {
        let root = TempDir::new("update-setting");
        let dir = root.path().to_path_buf();
        assert!(automatic_wanted(Some(&dir)), "no file");
        assert!(automatic_wanted(None));
        let write = |text: &str| std::fs::write(dir.join("config.toml"), text).unwrap();
        write("[general]\ntray = true\n");
        assert!(automatic_wanted(Some(&dir)));
        write("[general]\ncheck_updates = false\n[[widget]]\nkind = \"toaster\"\n");
        assert!(!automatic_wanted(Some(&dir)), "even if the layout is bad");
        write("[general]\ncheck_updates = true\n");
        assert!(automatic_wanted(Some(&dir)));
        write("this is not toml");
        assert!(automatic_wanted(Some(&dir)));
    }

    #[test]
    fn the_language_follows_the_file() {
        let root = TempDir::new("update-language");
        let dir = root.path().to_path_buf();
        let write = |text: &str| std::fs::write(dir.join("config.toml"), text).unwrap();
        write("[general]\nlanguage = \"italian\"\n");
        assert_eq!(language(Some(&dir)), Lang::Italian);
        write("[general]\nlanguage = \"english\"\n");
        assert_eq!(language(Some(&dir)), Lang::English);
    }

    #[test]
    fn every_message_exists_in_both_languages() {
        let release = parse_release(GOOD).unwrap();
        for what in [
            Outcome::Newer(release),
            Outcome::Current,
            Outcome::Failed("x".into()),
        ] {
            let en = words(Lang::English, &what);
            let it = words(Lang::Italian, &what);
            assert!(!en.0.is_empty() && !en.1.is_empty());
            assert!(!it.0.is_empty() && !it.1.is_empty());
            assert_ne!(en, it);
        }
        assert!(
            words(Lang::English, &Outcome::Newer(parse_release(GOOD).unwrap()))
                .0
                .contains("0.1.1")
        );
    }
}
