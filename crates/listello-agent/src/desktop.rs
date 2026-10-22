//! `.desktop` files: which application a Wayland `app_id` is, what its icon is called and how
//! to start it.
//!
//! The agent runs as the user, so it can read `~/.local/share/applications` and the user's
//! icon theme -- the reason icons cross the socket as pixels and not paths (D11).
//!
//! A window's `app_id` is *usually* its desktop file's id (`org.kde.dolphin`) and often is not
//! (`firefox` for `firefox-esr.desktop`, `jetbrains-idea` for `intellij-idea-ultimate`).
//! [`Catalog::find`] tries the honest matches in order and then the user's own table
//! (`~/.config/listello/apps.toml`), which wins over all of them.
//!
//! Everything here is external data -- files on the user's disk written by anyone -- so
//! parsing never panics and a malformed file is skipped, not fatal.

use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Files larger than this are not desktop files.
const MAX_DESKTOP_FILE_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopEntry {
    /// The file's name without `.desktop`, which is also the id the desktop uses for it.
    pub id: String,
    pub name: String,
    pub exec: Option<String>,
    pub icon: Option<String>,
    pub wm_class: Option<String>,
    pub terminal: bool,
}

/// Parse the `[Desktop Entry]` group. `None` for anything that is not a launchable
/// application: wrong `Type`, `Hidden=true`, or no group at all.
pub fn parse_entry(id: &str, text: &str) -> Option<DesktopEntry> {
    let mut in_group = false;
    let mut found_group = false;
    let mut fields: HashMap<&str, &str> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            found_group |= in_group;
            continue;
        }
        if !in_group {
            continue;
        }
        // Localised keys (`Name[it]`) are skipped; the unlocalised one is the fallback.
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            if !key.contains('[') {
                fields.entry(key).or_insert(value.trim());
            }
        }
    }
    if !found_group
        || fields.get("Type").copied() != Some("Application")
        || fields.get("Hidden").copied() == Some("true")
    {
        return None;
    }
    let text_field = |key: &str| {
        fields
            .get(key)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
    };
    Some(DesktopEntry {
        id: id.to_string(),
        name: text_field("Name").unwrap_or_else(|| id.to_string()),
        exec: text_field("Exec"),
        icon: text_field("Icon"),
        wm_class: text_field("StartupWMClass"),
        terminal: fields.get("Terminal").copied() == Some("true"),
    })
}

/// Split an `Exec=` line into an argument vector, per the Desktop Entry spec: double quotes
/// group, backslash escapes inside them, and the field codes (`%u`, `%F`, ...) are dropped
/// because we launch with no files. `%%` is a literal percent.
pub fn exec_argv(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut has_current = false;
    let mut in_quotes = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_current = true;
            }
            '\\' if in_quotes => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '%' => {
                // `%%` is a literal percent; any other code is a field code, with nothing to
                // substitute, and must not leave a stray token.
                if chars.next_if_eq(&'%').is_some() {
                    current.push('%');
                    has_current = true;
                } else {
                    chars.next();
                }
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_current || !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                    has_current = false;
                }
            }
            c => {
                current.push(c);
                has_current = true;
            }
        }
    }
    if has_current || !current.is_empty() {
        args.push(current);
    }
    args
}

/// The user's own mapping, for the apps whose `app_id` matches no desktop file.
#[derive(Default, Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    /// `app_id` -> desktop file id.
    #[serde(default)]
    pub apps: HashMap<String, String>,
    /// `app_id` -> icon name or absolute path, for an app with no usable icon of its own.
    #[serde(default)]
    pub icons: HashMap<String, String>,
}

impl Overrides {
    pub fn parse(text: &str) -> Result<Overrides, toml::de::Error> {
        toml::from_str(text)
    }

    /// Read `~/.config/listello/apps.toml`. A missing file is normal; a broken one is reported
    /// and ignored so a typo does not take the Dock down.
    pub fn load() -> Overrides {
        let Some(path) = config_dir().map(|d| d.join("listello/apps.toml")) else {
            return Overrides::default();
        };
        match fs::read_to_string(&path) {
            Ok(text) => Overrides::parse(&text).unwrap_or_else(|e| {
                eprintln!("listello-agent: ignoring {}: {e}", path.display());
                Overrides::default()
            }),
            Err(_) => Overrides::default(),
        }
    }
}

pub struct Catalog {
    entries: Vec<DesktopEntry>,
    overrides: Overrides,
}

impl Catalog {
    pub fn new(entries: Vec<DesktopEntry>, overrides: Overrides) -> Catalog {
        Catalog { entries, overrides }
    }

    /// Scan `applications/` under every data directory, user directory first so it wins.
    pub fn scan(overrides: Overrides) -> Catalog {
        let mut entries: Vec<DesktopEntry> = Vec::new();
        for dir in data_dirs() {
            collect(&dir.join("applications"), "", &mut entries);
        }
        // First occurrence of an id wins, and the directories are in precedence order.
        let mut seen = std::collections::HashSet::new();
        entries.retain(|e| seen.insert(e.id.clone()));
        Catalog { entries, overrides }
    }

    /// The desktop entry for a window's `app_id`.
    pub fn find(&self, app_id: &str) -> Option<&DesktopEntry> {
        if let Some(id) = self.overrides.apps.get(app_id) {
            if let Some(entry) = self.by_id(id) {
                return Some(entry);
            }
        }
        let lower = app_id.to_lowercase();
        let strip = |s: &str| s.strip_suffix(".desktop").unwrap_or(s).to_lowercase();
        // 1. The id itself, ignoring case: `org.kde.dolphin`, `Firefox`.
        self.entries
            .iter()
            .find(|e| e.id.to_lowercase() == lower)
            // 2. The last segment of a reverse-DNS id: `dolphin` for `org.kde.dolphin`.
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|e| e.id.rsplit('.').next().map(str::to_lowercase) == Some(lower.clone()))
            })
            // 3. The class the app says it sets.
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|e| e.wm_class.as_deref().map(strip) == Some(lower.clone()))
            })
            // 4. The program the entry runs.
            .or_else(|| {
                self.entries.iter().find(|e| {
                    e.exec
                        .as_deref()
                        .and_then(|x| exec_argv(x).into_iter().next())
                        .and_then(|p| p.rsplit('/').next().map(str::to_lowercase))
                        == Some(lower.clone())
                })
            })
    }

    /// Every launchable app, for choosing one to pin.
    pub fn entries(&self) -> &[DesktopEntry] {
        &self.entries
    }

    fn by_id(&self, id: &str) -> Option<&DesktopEntry> {
        let id = id.strip_suffix(".desktop").unwrap_or(id);
        self.entries.iter().find(|e| e.id == id)
    }

    /// The icon name (or absolute path) to try for an app, most specific first.
    pub fn icon_names(&self, app_id: &str) -> Vec<String> {
        let mut names = Vec::new();
        if let Some(icon) = self.overrides.icons.get(app_id) {
            names.push(icon.clone());
        }
        if let Some(icon) = self.find(app_id).and_then(|e| e.icon.clone()) {
            names.push(icon);
        }
        names.push(app_id.to_string());
        names
    }
}

/// Recursively read `.desktop` files; a file in `a/b.desktop` has the id `a-b`.
fn collect(dir: &Path, prefix: &str, out: &mut Vec<DesktopEntry>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    for item in read.flatten() {
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            collect(&path, &format!("{prefix}{name}-"), out);
        } else if let Some(stem) = name.strip_suffix(".desktop") {
            let too_big = item
                .metadata()
                .map(|m| m.len() > MAX_DESKTOP_FILE_BYTES)
                .unwrap_or(true);
            if too_big {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                if let Some(entry) = parse_entry(&format!("{prefix}{stem}"), &text) {
                    out.push(entry);
                }
            }
        }
    }
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home().map(|h| h.join(".config")))
}

/// `$XDG_DATA_HOME` then `$XDG_DATA_DIRS`, in the order the spec says they win.
pub fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(p) => dirs.push(PathBuf::from(p)),
        None => dirs.extend(home().map(|h| h.join(".local/share"))),
    }
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    dirs.extend(system.split(':').map(PathBuf::from));
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, body: &str) -> DesktopEntry {
        parse_entry(id, &format!("[Desktop Entry]\nType=Application\n{body}")).expect("an entry")
    }

    #[test]
    fn a_plain_entry_is_parsed() {
        let e = entry(
            "org.kde.dolphin",
            "Name=Dolphin\nName[it]=Dolphin IT\nExec=dolphin %u\nIcon=system-file-manager\n",
        );
        assert_eq!(e.name, "Dolphin");
        assert_eq!(e.icon.as_deref(), Some("system-file-manager"));
        assert!(!e.terminal);
    }

    #[test]
    fn hidden_and_non_application_entries_are_skipped() {
        assert!(parse_entry("a", "[Desktop Entry]\nType=Application\nHidden=true\n").is_none());
        assert!(parse_entry("a", "[Desktop Entry]\nType=Link\nURL=x\n").is_none());
        assert!(parse_entry("a", "no group at all").is_none());
        assert!(parse_entry("a", "").is_none());
    }

    #[test]
    fn keys_outside_the_main_group_are_ignored() {
        let text = "[Desktop Entry]\nType=Application\nName=Main\n\n\
                    [Desktop Action new]\nName=Other\nExec=other\n";
        let e = parse_entry("a", text).expect("an entry");
        assert_eq!(e.name, "Main");
        assert_eq!(e.exec, None);
    }

    #[test]
    fn exec_drops_field_codes_and_keeps_quoted_arguments() {
        assert_eq!(exec_argv("firefox %u"), ["firefox"]);
        assert_eq!(
            exec_argv(r#"env FOO=1 "/opt/My App/run" --flag %F"#),
            ["env", "FOO=1", "/opt/My App/run", "--flag"]
        );
        assert_eq!(exec_argv("printf 100%%"), ["printf", "100%"]);
        assert_eq!(
            exec_argv(r#"sh -c "echo \"hi\"""#),
            ["sh", "-c", "echo \"hi\""]
        );
        assert!(exec_argv("").is_empty());
        assert!(exec_argv("%U").is_empty());
    }

    fn catalog() -> Catalog {
        Catalog::new(
            vec![
                entry("org.kde.dolphin", "Name=Dolphin\nExec=dolphin %u\nIcon=system-file-manager"),
                entry("firefox-esr", "Name=Firefox\nExec=/usr/lib/firefox-esr/firefox %u\nIcon=firefox-esr\nStartupWMClass=Firefox"),
                entry("intellij-idea-ultimate", "Name=IDEA\nExec=idea\nIcon=idea"),
            ],
            Overrides {
                apps: HashMap::from([("jetbrains-idea".into(), "intellij-idea-ultimate".into())]),
                icons: HashMap::new(),
            },
        )
    }

    #[test]
    fn app_ids_find_their_entries_the_honest_ways() {
        let c = catalog();
        assert_eq!(c.find("org.kde.dolphin").unwrap().id, "org.kde.dolphin");
        assert_eq!(c.find("dolphin").unwrap().id, "org.kde.dolphin");
        assert_eq!(c.find("firefox").unwrap().id, "firefox-esr");
        assert_eq!(c.find("Firefox").unwrap().id, "firefox-esr");
        assert!(c.find("nothing-like-it").is_none());
    }

    #[test]
    fn the_users_table_wins() {
        assert_eq!(
            catalog().find("jetbrains-idea").unwrap().id,
            "intellij-idea-ultimate"
        );
    }

    #[test]
    fn icon_names_fall_back_to_the_app_id() {
        let c = catalog();
        assert_eq!(c.icon_names("dolphin"), ["system-file-manager", "dolphin"]);
        assert_eq!(c.icon_names("unknown-app"), ["unknown-app"]);
    }

    #[test]
    fn the_overrides_file_rejects_typos() {
        assert!(Overrides::parse("[apps]\nfoo = \"bar\"\n").is_ok());
        assert!(Overrides::parse("[aps]\nfoo = \"bar\"\n").is_err());
    }
}
