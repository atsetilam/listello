//! The layout being edited: the user's `config.toml` as a TOML document, plus the Dock's pins.
//!
//! Edited as a document rather than as a parsed `UiConfig`, so a hand-written file keeps its
//! comments and its order through the window. Every change is checked by parsing the text
//! with `UiConfig::parse` — the same check the agent and the renderer make — and the window
//! refuses to Apply a layout that fails it (D8 amendment).
//!
//! No Qt here, so all of it is tested without a display.

use listello_ui::UiConfig;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Item, Table, Value};

/// The strip, or what replaces it while Fn is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Strip,
    Fn,
}

impl Layer {
    fn key(self) -> &'static str {
        match self {
            Layer::Strip => "widget",
            Layer::Fn => "fn_widget",
        }
    }
}

/// The widget kinds the window offers to add, in the order it offers them.
pub const KINDS: [&str; 8] = [
    "dock",
    "folders",
    "control_center",
    "network",
    "battery",
    "clock",
    "button",
    "separator",
];

/// Where the files live.
#[derive(Clone, Debug)]
pub struct Paths {
    /// `~/.config/listello`: `config.toml` and `pins.toml`.
    pub user_dir: PathBuf,
    /// The machine's layouts, most specific first: `/etc/listello/config.toml`, then the
    /// packaged one.
    pub machine: Vec<PathBuf>,
}

impl Paths {
    pub fn user_config(&self) -> PathBuf {
        self.user_dir.join("config.toml")
    }

    pub fn user_pins(&self) -> PathBuf {
        self.user_dir.join("pins.toml")
    }
}

pub struct Model {
    paths: Paths,
    doc: DocumentMut,
    /// The text as last loaded or saved, to tell whether anything changed.
    saved_text: String,
    pins: Vec<String>,
    saved_pins: Vec<String>,
    /// Whether the user has a layout of their own on disk, or is seeing the machine's.
    pub has_own_layout: bool,
}

impl Model {
    /// The user's layout when there is one that parses as TOML, the machine's otherwise.
    pub fn load(paths: Paths) -> Model {
        let own = fs::read_to_string(paths.user_config())
            .ok()
            .filter(|t| t.parse::<DocumentMut>().is_ok());
        let has_own_layout = own.is_some();
        let text = own.unwrap_or_else(|| machine_text(&paths));
        let mut doc = text.parse::<DocumentMut>().unwrap_or_default();
        // `[core]` belongs to the device, never to a user (D8 amendment).
        doc.remove("core");
        let saved_text = doc.to_string();
        let pins = listello_agent::pins::load(&paths.user_pins())
            .map(|p| p.into_iter().map(|a| a.0).collect())
            .unwrap_or_else(|| config_pins(&doc));
        Model {
            paths,
            doc,
            saved_text,
            saved_pins: pins.clone(),
            pins,
            has_own_layout,
        }
    }

    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    /// The layout as the Touch Bar would read it, or why it would refuse it.
    pub fn validate(&self) -> Result<UiConfig, String> {
        UiConfig::parse(&self.text()).map_err(|e| e.to_string())
    }

    pub fn is_dirty(&self) -> bool {
        self.text() != self.saved_text || self.pins != self.saved_pins
    }

    /// Whether Apply would change what the Touch Bar shows. The tray switch lives in the same
    /// file but is not the Touch Bar's, so a change to it alone does not count.
    pub fn changes_touch_bar(&self) -> bool {
        // With no edits at all, Apply on the machine's layout makes it the user's: say so.
        let untouched = self.text() == self.saved_text && self.pins == self.saved_pins;
        (untouched && !self.has_own_layout)
            || self.pins != self.saved_pins
            || without_tray(&self.text()) != without_tray(&self.saved_text)
    }

    /// Write the layout (and the pins, when they changed) where the agent watches for them.
    /// Refuses a layout that does not validate.
    pub fn apply(&mut self) -> Result<(), String> {
        self.validate()?;
        let text = self.text();
        write_atomically(&self.paths.user_config(), &text).map_err(|e| e.to_string())?;
        if self.pins != self.saved_pins {
            let pins: Vec<_> = self
                .pins
                .iter()
                .map(|p| listello_protocol::AppId::new(p.clone()))
                .collect();
            listello_agent::pins::save(&self.paths.user_pins(), &pins)
                .map_err(|e| e.to_string())?;
            self.saved_pins = self.pins.clone();
        }
        self.saved_text = text;
        self.has_own_layout = true;
        Ok(())
    }

    /// Drop the edits: back to what is on disk.
    pub fn cancel(&mut self) {
        *self = Model::load(self.paths.clone());
    }

    /// Drop this user's layout and pins, so the machine's apply again.
    pub fn restore_default(&mut self) -> Result<(), String> {
        for path in [self.paths.user_config(), self.paths.user_pins()] {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        *self = Model::load(self.paths.clone());
        Ok(())
    }

    // ---- widgets ------------------------------------------------------------------------

    pub fn count(&self, layer: Layer) -> usize {
        self.widgets(layer).map_or(0, ArrayOfTables::len)
    }

    pub fn kind(&self, layer: Layer, index: usize) -> String {
        self.widget(layer, index)
            .and_then(|t| t.get("kind"))
            .and_then(Item::as_str)
            .unwrap_or_default()
            .to_string()
    }

    /// Add a widget of `kind` at `at` (clamped), with what it needs to be useful. Returns
    /// where it went.
    pub fn add(&mut self, layer: Layer, kind: &str, at: usize) -> usize {
        let mut tables = self.take_widgets(layer);
        let at = at.min(tables.len());
        tables.insert(at, new_widget(kind));
        self.put_widgets(layer, tables);
        at
    }

    pub fn remove(&mut self, layer: Layer, index: usize) {
        let mut tables = self.take_widgets(layer);
        if index < tables.len() {
            tables.remove(index);
        }
        self.put_widgets(layer, tables);
    }

    /// Move widget `from` so it lands at position `to` of the list without it.
    pub fn move_to(&mut self, layer: Layer, from: usize, to: usize) {
        let mut tables = self.take_widgets(layer);
        if from < tables.len() {
            let table = tables.remove(from);
            tables.insert(to.min(tables.len()), table);
        }
        self.put_widgets(layer, tables);
    }

    pub fn get_str(&self, layer: Layer, index: usize, key: &str) -> Option<String> {
        self.widget(layer, index)?
            .get(key)?
            .as_str()
            .map(str::to_string)
    }

    pub fn get_bool(&self, layer: Layer, index: usize, key: &str) -> Option<bool> {
        self.widget(layer, index)?.get(key)?.as_bool()
    }

    pub fn get_f64(&self, layer: Layer, index: usize, key: &str) -> Option<f64> {
        number(self.widget(layer, index)?.get(key)?)
    }

    /// A list of strings (a button's `keys`), joined with ", " for a text field.
    pub fn get_list(&self, layer: Layer, index: usize, key: &str) -> String {
        self.widget(layer, index)
            .and_then(|t| t.get(key))
            .map(join_list)
            .unwrap_or_default()
    }

    /// Set a text field; an empty one is removed, so the default applies.
    pub fn set_str(&mut self, layer: Layer, index: usize, key: &str, text: &str) {
        if let Some(t) = self.widget_mut(layer, index) {
            set_str_in(t, key, text);
        }
    }

    pub fn set_bool(&mut self, layer: Layer, index: usize, key: &str, on: bool) {
        if let Some(t) = self.widget_mut(layer, index) {
            t.insert(key, value(on));
        }
    }

    /// Set a number, or remove it with `None`.
    pub fn set_f64(&mut self, layer: Layer, index: usize, key: &str, number: Option<f64>) {
        if let Some(t) = self.widget_mut(layer, index) {
            set_number_in(t, key, number);
        }
    }

    /// Set a list of strings from "F5, VolumeUp"; an empty one is removed.
    pub fn set_list(&mut self, layer: Layer, index: usize, key: &str, text: &str) {
        if let Some(t) = self.widget_mut(layer, index) {
            set_list_in(t, key, text);
        }
    }

    /// How wide a widget is: `"fixed"` with a width, `"share"` with a flex, or `"auto"`.
    pub fn sizing(&self, layer: Layer, index: usize) -> (String, f64) {
        if let Some(w) = self.get_f64(layer, index, "width") {
            ("fixed".into(), w)
        } else if let Some(f) = self.get_f64(layer, index, "flex") {
            ("share".into(), f)
        } else {
            ("auto".into(), 0.0)
        }
    }

    pub fn set_sizing(&mut self, layer: Layer, index: usize, mode: &str, amount: f64) {
        let (width, flex) = match mode {
            "fixed" => (Some(amount.max(1.0)), None),
            "share" => (None, Some(amount.max(0.1))),
            _ => (None, None),
        };
        self.set_f64(layer, index, "width", width);
        self.set_f64(layer, index, "flex", flex);
    }

    // ---- a widget's own list: the control centre's buttons, the folders ------------------

    pub fn item_count(&self, layer: Layer, index: usize, list: &str) -> usize {
        self.widget(layer, index)
            .and_then(|t| t.get(list))
            .and_then(Item::as_array_of_tables)
            .map_or(0, ArrayOfTables::len)
    }

    pub fn item_str(
        &self,
        layer: Layer,
        index: usize,
        list: &str,
        item: usize,
        key: &str,
    ) -> String {
        self.item(layer, index, list, item)
            .and_then(|t| t.get(key))
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| join_list(v))
            })
            .unwrap_or_default()
    }

    pub fn set_item_str(
        &mut self,
        layer: Layer,
        index: usize,
        list: &str,
        item: usize,
        key: &str,
        text: &str,
    ) {
        let Some(t) = self.item_mut(layer, index, list, item) else {
            return;
        };
        match key {
            "keys" => set_list_in(t, key, text),
            // A folder's key and path and a button's icon are required: keep what was there
            // rather than write a file the Touch Bar would refuse over one emptied field.
            "key" | "path" | "icon" if text.trim().is_empty() => {}
            _ => set_str_in(t, key, text),
        }
        if key == "system" {
            t.remove("keys");
        }
        if key == "keys" && !text.trim().is_empty() {
            t.remove("system");
        }
    }

    pub fn add_item(&mut self, layer: Layer, index: usize, list: &str) {
        let Some(widget) = self.widget_mut(layer, index) else {
            return;
        };
        let taken: Vec<String> = widget
            .get(list)
            .and_then(Item::as_array_of_tables)
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.get("key").and_then(Item::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut table = Table::new();
        if list == "folder" {
            let key = (1..)
                .map(|n| format!("folder{n}"))
                .find(|k| !taken.contains(k))
                .unwrap_or_default();
            table.insert("key", value(key));
            table.insert("path", value("~"));
            table.insert("label", value("Home"));
            table.insert("icon", value("folder"));
        } else {
            table.insert("icon", value("cc-volume-mute"));
            table.insert("label", value("Mute"));
            table.insert("keys", value(Array::from_iter(["Mute"])));
        }
        match widget.get_mut(list).and_then(Item::as_array_of_tables_mut) {
            Some(items) => items.push(table),
            None => {
                let mut items = ArrayOfTables::new();
                items.push(table);
                widget.insert(list, Item::ArrayOfTables(items));
            }
        }
        self.renumber();
    }

    pub fn remove_item(&mut self, layer: Layer, index: usize, list: &str, item: usize) {
        self.edit_items(layer, index, list, |items| {
            if item < items.len() {
                items.remove(item);
            }
        });
    }

    pub fn move_item(&mut self, layer: Layer, index: usize, list: &str, from: usize, to: usize) {
        self.edit_items(layer, index, list, |items| {
            if from < items.len() {
                let t = items.remove(from);
                items.insert(to.min(items.len()), t);
            }
        });
    }

    // ---- the Dock's pins ------------------------------------------------------------------

    pub fn pins(&self) -> &[String] {
        &self.pins
    }

    pub fn set_pins(&mut self, pins: Vec<String>) {
        let mut clean: Vec<String> = Vec::new();
        for p in pins {
            let p = p.trim().to_string();
            if !p.is_empty() && !clean.contains(&p) {
                clean.push(p);
            }
        }
        self.pins = clean;
    }

    // ---- [strip] and [general] -----------------------------------------------------------

    pub fn section_str(&self, section: &str, key: &str) -> Option<String> {
        self.doc
            .get(section)?
            .get(key)?
            .as_str()
            .map(str::to_string)
    }

    pub fn section_f64(&self, section: &str, key: &str) -> Option<f64> {
        number(self.doc.get(section)?.get(key)?)
    }

    /// Set `[section] key`; an empty text or `None` removes it, so the default applies.
    pub fn set_section_str(&mut self, section: &str, key: &str, text: &str) {
        set_str_in(self.section_mut(section), key, text);
        self.drop_empty_section(section);
    }

    pub fn section_bool(&self, section: &str, key: &str) -> Option<bool> {
        self.doc.get(section)?.get(key)?.as_bool()
    }

    /// `false` removes it: every switch in the file is off by default.
    pub fn set_section_bool(&mut self, section: &str, key: &str, on: bool) {
        let t = self.section_mut(section);
        if on {
            if t.get(key).and_then(Item::as_bool) != Some(true) {
                t.insert(key, value(true));
            }
        } else {
            t.remove(key);
        }
        self.drop_empty_section(section);
    }

    pub fn set_section_f64(&mut self, section: &str, key: &str, number: Option<f64>) {
        set_number_in(self.section_mut(section), key, number);
        self.drop_empty_section(section);
    }

    // ---- plumbing -------------------------------------------------------------------------

    fn widgets(&self, layer: Layer) -> Option<&ArrayOfTables> {
        self.doc.get(layer.key())?.as_array_of_tables()
    }

    fn widget(&self, layer: Layer, index: usize) -> Option<&Table> {
        self.widgets(layer)?.get(index)
    }

    fn widget_mut(&mut self, layer: Layer, index: usize) -> Option<&mut Table> {
        self.doc
            .get_mut(layer.key())?
            .as_array_of_tables_mut()?
            .get_mut(index)
    }

    fn item(&self, layer: Layer, index: usize, list: &str, item: usize) -> Option<&Table> {
        self.widget(layer, index)?
            .get(list)?
            .as_array_of_tables()?
            .get(item)
    }

    fn item_mut(
        &mut self,
        layer: Layer,
        index: usize,
        list: &str,
        item: usize,
    ) -> Option<&mut Table> {
        self.widget_mut(layer, index)?
            .get_mut(list)?
            .as_array_of_tables_mut()?
            .get_mut(item)
    }

    fn edit_items(
        &mut self,
        layer: Layer,
        index: usize,
        list: &str,
        edit: impl FnOnce(&mut Vec<Table>),
    ) {
        let Some(widget) = self.widget_mut(layer, index) else {
            return;
        };
        let mut items: Vec<Table> = match widget.remove(list) {
            Some(Item::ArrayOfTables(a)) => a.into_iter().collect(),
            _ => Vec::new(),
        };
        edit(&mut items);
        if !items.is_empty() {
            widget.insert(list, Item::ArrayOfTables(items.into_iter().collect()));
        }
        self.renumber();
    }

    fn take_widgets(&mut self, layer: Layer) -> Vec<Table> {
        match self.doc.remove(layer.key()) {
            Some(Item::ArrayOfTables(a)) => a.into_iter().collect(),
            _ => Vec::new(),
        }
    }

    fn put_widgets(&mut self, layer: Layer, tables: Vec<Table>) {
        if !tables.is_empty() {
            self.doc.insert(
                layer.key(),
                Item::ArrayOfTables(tables.into_iter().collect()),
            );
        }
        self.renumber();
    }

    fn section_mut(&mut self, section: &str) -> &mut Table {
        if !self.doc.contains_table(section) {
            let mut table = Table::new();
            table.set_position(0);
            self.doc.insert(section, Item::Table(table));
        }
        let table = self.doc[section]
            .as_table_mut()
            .expect("just made sure it is a table");
        table.set_implicit(false);
        table
    }

    fn drop_empty_section(&mut self, section: &str) {
        if self
            .doc
            .get(section)
            .and_then(Item::as_table)
            .is_some_and(Table::is_empty)
        {
            self.doc.remove(section);
        }
        self.renumber();
    }

    /// Give every table its place in the file: `[general]`, `[strip]`, the widgets, the Fn
    /// widgets, then anything else, each followed by its own sub-tables. toml_edit writes
    /// tables in position order, and tables moved or made here would otherwise keep where
    /// they came from.
    fn renumber(&mut self) {
        let order = ["general", "strip", "widget", "fn_widget"];
        let mut keys: Vec<String> = order.iter().map(|k| k.to_string()).collect();
        for (k, _) in self.doc.iter() {
            if !order.contains(&k) {
                keys.push(k.to_string());
            }
        }
        let mut next = 1;
        for key in keys {
            if let Some(item) = self.doc.get_mut(&key) {
                number_item(item, &mut next);
            }
        }
    }
}

fn number_item(item: &mut Item, next: &mut usize) {
    match item {
        Item::Table(t) => number_table(t, next),
        Item::ArrayOfTables(a) => {
            for t in a.iter_mut() {
                number_table(t, next);
            }
        }
        _ => {}
    }
}

fn number_table(table: &mut Table, next: &mut usize) {
    table.set_position(*next);
    *next += 1;
    for (_, item) in table.iter_mut() {
        number_item(item, next);
    }
}

fn machine_text(paths: &Paths) -> String {
    paths
        .machine
        .iter()
        .find_map(|p| fs::read_to_string(p).ok())
        .unwrap_or_default()
}

/// The first Dock's `pinned`, the list the Touch Bar shows until the user pins something.
fn config_pins(doc: &DocumentMut) -> Vec<String> {
    doc.get("widget")
        .and_then(Item::as_array_of_tables)
        .and_then(|a| {
            a.iter()
                .find(|t| t.get("kind").and_then(Item::as_str) == Some("dock"))
        })
        .and_then(|t| t.get("pinned"))
        .and_then(Item::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn new_widget(kind: &str) -> Table {
    let mut t = Table::new();
    t.insert("kind", value(kind));
    match kind {
        "button" => {
            t.insert("label", value("F1"));
            t.insert("keys", value(Array::from_iter(["F1"])));
        }
        "clock" => {
            t.insert("format", value("%H:%M"));
        }
        "separator" => {}
        "battery" => {
            t.insert("width", value(112.0));
        }
        "network" => {
            t.insert("width", value(56.0));
        }
        "dock" => {
            t.insert("flex", value(1.0));
        }
        "folders" => {
            let mut folder = Table::new();
            folder.insert("key", value("downloads"));
            folder.insert("path", value("~/Downloads"));
            folder.insert("label", value("Down"));
            folder.insert("icon", value("folder-downloads"));
            let mut items = ArrayOfTables::new();
            items.push(folder);
            t.insert("width", value(92.0));
            t.insert("folder", Item::ArrayOfTables(items));
        }
        "control_center" => {
            let mut items = ArrayOfTables::new();
            for (icon, label, key) in [
                ("cc-volume-down", "Vol-", "VolumeDown"),
                ("cc-volume-up", "Vol+", "VolumeUp"),
            ] {
                let mut b = Table::new();
                b.insert("icon", value(icon));
                b.insert("label", value(label));
                b.insert("keys", value(Array::from_iter([key])));
                items.push(b);
            }
            t.insert("width", value(228.0));
            t.insert("button", Item::ArrayOfTables(items));
        }
        _ => {}
    }
    t
}

fn number(item: &Item) -> Option<f64> {
    item.as_float()
        .or_else(|| item.as_integer().map(|i| i as f64))
}

fn join_list(item: &Item) -> String {
    item.as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// The file as values, minus `[general] tray`; `None` when it does not parse.
fn without_tray(text: &str) -> Option<toml::Table> {
    let mut table: toml::Table = text.parse().ok()?;
    if let Some(general) = table.get_mut("general").and_then(toml::Value::as_table_mut) {
        general.remove("tray");
        if general.is_empty() {
            table.remove("general");
        }
    }
    Some(table)
}

fn set_str_in(t: &mut Table, key: &str, text: &str) {
    if text.is_empty() {
        t.remove(key);
    } else if t.get(key).and_then(Item::as_str) != Some(text) {
        t.insert(key, value(text));
    }
}

fn set_number_in(t: &mut Table, key: &str, number: Option<f64>) {
    match number.filter(|n| n.is_finite()) {
        None => {
            t.remove(key);
        }
        Some(n) if t.get(key).and_then(self::number) != Some(n) => {
            t.insert(key, value(n));
        }
        Some(_) => {}
    }
}

fn set_list_in(t: &mut Table, key: &str, text: &str) {
    let items: Vec<&str> = text
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if items.is_empty() {
        t.remove(key);
    } else {
        t.insert(key, value(Array::from_iter(items)));
    }
}

fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    {
        let mut out = fs::File::create(&tmp)?;
        out.write_all(text.as_bytes())?;
        out.sync_all()?;
    }
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MACHINE: &str = "\
# The machine's layout.
[strip]
gap = 12.0

# The Dock first.
[[widget]]
kind = \"dock\"
pinned = [\"org.kde.dolphin\", \"firefox\"]
flex = 1.0

[[widget]]
kind = \"control_center\"
width = 200.0

[[widget.button]]
icon = \"cc-lock\"
system = \"lock_session\"

[[widget]]
kind = \"clock\"
locale = \"it_IT\"

[core]
active_brightness = 128
";

    fn setup(name: &str) -> Paths {
        let root =
            std::env::temp_dir().join(format!("listello-settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("machine.toml"), MACHINE).unwrap();
        Paths {
            user_dir: root.join("user"),
            machine: vec![root.join("missing.toml"), root.join("machine.toml")],
        }
    }

    #[test]
    fn with_no_own_layout_the_machines_is_the_start_without_its_core() {
        let m = Model::load(setup("start"));
        assert!(!m.has_own_layout);
        assert!(!m.text().contains("[core]"));
        assert!(m.text().contains("# The Dock first."));
        assert_eq!(m.count(Layer::Strip), 3);
        assert_eq!(m.pins(), ["org.kde.dolphin", "firefox"]);
        assert!(!m.is_dirty());
        assert!(m.validate().is_ok());
    }

    #[test]
    fn turning_the_tray_on_or_off_alone_does_not_change_the_touch_bar() {
        let mut m = Model::load(setup("tray"));
        assert!(
            m.changes_touch_bar(),
            "Apply would make the machine's layout the user's"
        );
        m.set_section_bool("general", "tray", true);
        assert!(m.is_dirty());
        assert!(!m.changes_touch_bar());
        m.apply().unwrap();
        m.set_section_bool("general", "tray", false);
        assert!(!m.changes_touch_bar());
        m.set_section_f64("strip", "gap", Some(4.0));
        assert!(m.changes_touch_bar());
    }

    #[test]
    fn moving_a_widget_moves_its_comment_and_sub_tables_with_it() {
        let mut m = Model::load(setup("move"));
        m.move_to(Layer::Strip, 1, 0);
        assert_eq!(m.kind(Layer::Strip, 0), "control_center");
        assert_eq!(m.kind(Layer::Strip, 1), "dock");
        let text = m.text();
        let cc = text.find("control_center").unwrap();
        let button = text.find("[[widget.button]]").unwrap();
        let dock = text.find("kind = \"dock\"").unwrap();
        assert!(cc < button && button < dock, "{text}");
        assert!(text.find("# The Dock first.").unwrap() < dock);
        assert!(m.is_dirty());
        // And it reads back the same way.
        let cfg = m.validate().unwrap();
        assert_eq!(cfg.widgets.len(), 3);
    }

    #[test]
    fn every_kind_the_window_offers_validates_as_added() {
        let mut m = Model::load(setup("kinds"));
        for kind in KINDS {
            let at = m.add(Layer::Fn, kind, usize::MAX);
            assert_eq!(m.kind(Layer::Fn, at), kind);
            m.validate().unwrap_or_else(|e| panic!("{kind}: {e}"));
        }
        assert_eq!(m.count(Layer::Fn), KINDS.len());
        assert_eq!(m.count(Layer::Strip), 3, "the strip is left alone");
        // The Fn widgets come after the strip's in the file.
        let text = m.text();
        assert!(text.rfind("[[widget]]").unwrap() < text.find("[[fn_widget]]").unwrap());
    }

    #[test]
    fn fields_set_read_back_and_an_emptied_one_goes_back_to_its_default() {
        let mut m = Model::load(setup("fields"));
        m.set_str(Layer::Strip, 2, "format", "%a %H:%M");
        assert_eq!(
            m.get_str(Layer::Strip, 2, "format").as_deref(),
            Some("%a %H:%M")
        );
        m.set_str(Layer::Strip, 2, "format", "");
        assert_eq!(m.get_str(Layer::Strip, 2, "format"), None);
        m.set_sizing(Layer::Strip, 2, "fixed", 300.0);
        assert_eq!(m.sizing(Layer::Strip, 2), ("fixed".into(), 300.0));
        m.set_sizing(Layer::Strip, 2, "auto", 0.0);
        assert_eq!(m.sizing(Layer::Strip, 2).0, "auto");
        let at = m.add(Layer::Strip, "button", 0);
        m.set_list(Layer::Strip, at, "keys", "LeftCtrl, C ,");
        assert_eq!(m.get_list(Layer::Strip, at, "keys"), "LeftCtrl, C");
        assert!(m.validate().is_ok());
    }

    #[test]
    fn a_bad_value_is_refused_at_apply_and_nothing_is_written() {
        let paths = setup("refuse");
        let mut m = Model::load(paths.clone());
        m.set_section_f64("strip", "gap", Some(-3.0));
        assert!(m.validate().is_err());
        assert!(m.apply().is_err());
        assert!(!paths.user_config().exists());
    }

    #[test]
    fn apply_writes_the_layout_and_the_pins_and_restore_takes_both_away() {
        let paths = setup("apply");
        let mut m = Model::load(paths.clone());
        m.set_section_str("general", "language", "english");
        m.set_pins(vec!["firefox".into(), " ".into(), "firefox".into()]);
        m.apply().unwrap();
        assert!(!m.is_dirty());
        let written = fs::read_to_string(paths.user_config()).unwrap();
        assert!(
            written.starts_with("[general]\nlanguage = \"english\""),
            "{written}"
        );
        assert!(UiConfig::parse(&written).is_ok());

        let again = Model::load(paths.clone());
        assert!(again.has_own_layout);
        assert_eq!(again.pins(), ["firefox"]);
        assert_eq!(
            again.section_str("general", "language").as_deref(),
            Some("english")
        );

        m.restore_default().unwrap();
        assert!(!paths.user_config().exists() && !paths.user_pins().exists());
        assert!(!m.has_own_layout);
        assert_eq!(m.section_str("general", "language"), None);
    }

    #[test]
    fn cancel_drops_the_edits() {
        let mut m = Model::load(setup("cancel"));
        m.remove(Layer::Strip, 0);
        assert!(m.is_dirty());
        m.cancel();
        assert!(!m.is_dirty());
        assert_eq!(m.count(Layer::Strip), 3);
    }

    #[test]
    fn a_widgets_own_list_can_grow_shrink_and_reorder() {
        let mut m = Model::load(setup("items"));
        m.add_item(Layer::Strip, 1, "button");
        assert_eq!(m.item_count(Layer::Strip, 1, "button"), 2);
        m.set_item_str(Layer::Strip, 1, "button", 1, "keys", "VolumeUp");
        m.move_item(Layer::Strip, 1, "button", 1, 0);
        assert_eq!(m.item_str(Layer::Strip, 1, "button", 0, "keys"), "VolumeUp");
        // Choosing the system action drops the keys, and the other way round.
        m.set_item_str(Layer::Strip, 1, "button", 0, "system", "lock_session");
        assert_eq!(m.item_str(Layer::Strip, 1, "button", 0, "keys"), "");
        assert!(m.validate().is_ok());
        m.remove_item(Layer::Strip, 1, "button", 0);
        m.remove_item(Layer::Strip, 1, "button", 0);
        assert_eq!(m.item_count(Layer::Strip, 1, "button"), 0);

        let at = m.add(Layer::Strip, "folders", 0);
        m.add_item(Layer::Strip, at, "folder");
        assert_eq!(m.item_str(Layer::Strip, at, "folder", 1, "key"), "folder1");
        m.set_item_str(Layer::Strip, at, "folder", 1, "path", "");
        assert_eq!(m.item_str(Layer::Strip, at, "folder", 1, "path"), "~");
        assert!(m.validate().is_ok());
    }
}
