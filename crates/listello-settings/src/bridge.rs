//! What the QML window sees: one object, `Settings`, over the [`Model`] and the [`Painter`].
//!
//! QML reads through invokables rather than one property per field: the set of fields
//! depends on the widget picked. Every change bumps `revision`, and the QML bindings that read
//! a field mention `revision`, so they all re-read after any change.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
    }

    unsafe extern "C++" {
        include!("listello-settings/cpp/app.h");
        fn run_settings_window() -> i32;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, picture)]
        #[qproperty(f64, panel_width)]
        #[qproperty(f64, panel_height)]
        #[qproperty(i32, layer)]
        #[qproperty(i32, selected)]
        #[qproperty(i32, revision)]
        #[qproperty(i32, widget_count)]
        #[qproperty(QString, error)]
        #[qproperty(bool, dirty)]
        #[qproperty(bool, own_layout)]
        #[qproperty(QString, ui_lang)]
        type Settings = super::SettingsRust;

        #[qinvokable]
        fn refresh(self: Pin<&mut Settings>);
        #[qinvokable]
        fn tr(self: &Settings, key: &QString) -> QString;
        #[qinvokable]
        fn start_view(self: &Settings) -> QList_i32;
        #[qinvokable]
        fn set_strip_layer(self: Pin<&mut Settings>, layer: i32);

        // The picture.
        #[qinvokable]
        fn widget_at(self: &Settings, x: f64) -> i32;
        #[qinvokable]
        fn span_x(self: &Settings, index: i32) -> f64;
        #[qinvokable]
        fn span_width(self: &Settings, index: i32) -> f64;
        #[qinvokable]
        fn drop_slot(self: &Settings, x: f64) -> i32;
        #[qinvokable]
        fn slot_x(self: &Settings, slot: i32) -> f64;

        // Widgets.
        #[qinvokable]
        fn kind(self: &Settings, index: i32) -> QString;
        #[qinvokable]
        fn kind_count(self: &Settings) -> i32;
        #[qinvokable]
        fn kind_at(self: &Settings, i: i32) -> QString;
        #[qinvokable]
        fn add_widget(self: Pin<&mut Settings>, kind: &QString);
        #[qinvokable]
        fn remove_selected(self: Pin<&mut Settings>);
        #[qinvokable]
        fn move_widget(self: Pin<&mut Settings>, from: i32, slot: i32);
        #[qinvokable]
        fn get_str(self: &Settings, key: &QString) -> QString;
        #[qinvokable]
        fn set_str(self: Pin<&mut Settings>, key: &QString, text: &QString);
        #[qinvokable]
        fn get_bool(self: &Settings, key: &QString, fallback: bool) -> bool;
        #[qinvokable]
        fn set_bool(self: Pin<&mut Settings>, key: &QString, on: bool);
        #[qinvokable]
        fn get_list(self: &Settings, key: &QString) -> QString;
        #[qinvokable]
        fn set_list(self: Pin<&mut Settings>, key: &QString, text: &QString);
        #[qinvokable]
        fn sizing_mode(self: &Settings) -> QString;
        #[qinvokable]
        fn sizing_amount(self: &Settings) -> f64;
        #[qinvokable]
        fn set_sizing(self: Pin<&mut Settings>, mode: &QString, amount: f64);

        // A widget's own list.
        #[qinvokable]
        fn item_count(self: &Settings, list: &QString) -> i32;
        #[qinvokable]
        fn item_str(self: &Settings, list: &QString, item: i32, key: &QString) -> QString;
        #[qinvokable]
        fn set_item_str(
            self: Pin<&mut Settings>,
            list: &QString,
            item: i32,
            key: &QString,
            text: &QString,
        );
        #[qinvokable]
        fn add_item(self: Pin<&mut Settings>, list: &QString);
        #[qinvokable]
        fn remove_item(self: Pin<&mut Settings>, list: &QString, item: i32);
        #[qinvokable]
        fn move_item(self: Pin<&mut Settings>, list: &QString, from: i32, to: i32);

        // Packaged icons.
        #[qinvokable]
        fn icon_count(self: &Settings, context: &QString) -> i32;
        #[qinvokable]
        fn icon_name(self: &Settings, context: &QString, i: i32) -> QString;

        // Keys, and what a date format looks like.
        #[qinvokable]
        fn key_from_scan_code(self: &Settings, scan: i32) -> QString;
        #[qinvokable]
        fn key_label(self: &Settings, names: &QString) -> QString;
        #[qinvokable]
        fn special_key_count(self: &Settings) -> i32;
        #[qinvokable]
        fn special_key(self: &Settings, i: i32) -> QString;
        #[qinvokable]
        fn format_example(self: &Settings, format: &QString) -> QString;
        #[qinvokable]
        fn icon_url(self: &Settings, name: &QString) -> QString;

        // The Dock's pins, and the apps there are to pin.
        #[qinvokable]
        fn pin_count(self: &Settings) -> i32;
        #[qinvokable]
        fn pin_name(self: &Settings, i: i32) -> QString;
        #[qinvokable]
        fn pin_icon(self: Pin<&mut Settings>, i: i32) -> QString;
        #[qinvokable]
        fn add_pin(self: Pin<&mut Settings>, app: i32);
        #[qinvokable]
        fn remove_pin(self: Pin<&mut Settings>, i: i32);
        #[qinvokable]
        fn move_pin(self: Pin<&mut Settings>, from: i32, to: i32);
        #[qinvokable]
        fn app_count(self: &Settings) -> i32;
        #[qinvokable]
        fn app_name(self: &Settings, i: i32) -> QString;

        // [general] and [strip].
        #[qinvokable]
        fn section_str(self: &Settings, section: &QString, key: &QString) -> QString;
        #[qinvokable]
        fn set_section_str(
            self: Pin<&mut Settings>,
            section: &QString,
            key: &QString,
            text: &QString,
        );
        #[qinvokable]
        fn section_num(self: &Settings, section: &QString, key: &QString, fallback: f64) -> f64;
        #[qinvokable]
        fn set_section_num(self: Pin<&mut Settings>, section: &QString, key: &QString, number: f64);
        #[qinvokable]
        fn section_bool(self: &Settings, section: &QString, key: &QString) -> bool;
        #[qinvokable]
        fn set_section_bool(self: Pin<&mut Settings>, section: &QString, key: &QString, on: bool);
        #[qinvokable]
        fn check_updates(self: &Settings) -> bool;
        #[qinvokable]
        fn set_check_updates(self: Pin<&mut Settings>, on: bool);
        #[qinvokable]
        fn reload_listello(self: &Settings);
        #[qinvokable]
        fn stop_listello(self: &Settings);
        #[qinvokable]
        fn start_at_login(self: &Settings) -> bool;
        #[qinvokable]
        fn set_start_at_login(self: &Settings, on: bool);

        // The buttons at the bottom.
        #[qinvokable]
        fn changes_touch_bar(self: &Settings) -> bool;
        #[qinvokable]
        fn apply(self: Pin<&mut Settings>) -> bool;
        #[qinvokable]
        fn cancel(self: Pin<&mut Settings>);
        #[qinvokable]
        fn restore_default(self: Pin<&mut Settings>) -> bool;
    }
}

use crate::keys;
use crate::lifecycle;
use crate::model::{Layer, Model, Paths, KINDS};
use crate::picture::{Painter, Picture};
use crate::strings::{self, Lang};
use crate::tray;
use core::pin::Pin;
use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;
use listello_ui::UiConfig;
use std::path::PathBuf;

pub struct SettingsRust {
    picture: QString,
    panel_width: f64,
    panel_height: f64,
    layer: i32,
    selected: i32,
    revision: i32,
    widget_count: i32,
    error: QString,
    dirty: bool,
    own_layout: bool,
    ui_lang: QString,

    model: Model,
    painter: Painter,
    shown: Picture,
    /// The last layout that validated: the picture keeps showing it while an edit is invalid.
    last_good: Option<UiConfig>,
    share: PathBuf,
    icons: Vec<String>,
    /// Installed apps, by name, for pinning.
    apps: Vec<(String, String)>,
    time_locale: Option<String>,
}

impl Default for SettingsRust {
    fn default() -> Self {
        let share = std::env::var_os("LISTELLO_SHARE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/usr/share/listello"));
        let user_dir = listello_agent::desktop::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("listello");
        let model = Model::load(Paths {
            user_dir,
            machine: vec![
                PathBuf::from("/etc/listello/config.toml"),
                share.join("config.toml"),
            ],
        });
        let painter = Painter::new(share.clone());
        let mut apps: Vec<(String, String)> = painter
            .catalog()
            .entries()
            .iter()
            .map(|e| (e.id.clone(), e.name.clone()))
            .collect();
        apps.sort_by_key(|(_, name)| name.to_lowercase());
        let mut icons: Vec<String> = std::fs::read_dir(&share)
            .map(|d| {
                d.flatten()
                    .filter_map(|e| {
                        e.file_name()
                            .to_str()?
                            .strip_suffix(".svg")
                            .map(|n| n.replace('_', "-"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        icons.sort();
        let time_locale = ["LC_ALL", "LC_TIME", "LANG"]
            .iter()
            .filter_map(|n| std::env::var(n).ok())
            .find(|v| !v.is_empty());
        SettingsRust {
            picture: QString::default(),
            panel_width: painter.panel.width as f64,
            panel_height: painter.panel.height as f64,
            layer: 0,
            selected: -1,
            revision: 0,
            widget_count: 0,
            error: QString::default(),
            dirty: false,
            own_layout: model.has_own_layout,
            ui_lang: QString::default(),
            model,
            painter,
            shown: Picture {
                png: Vec::new(),
                spans: Vec::new(),
                targets: Vec::new(),
            },
            last_good: None,
            share,
            icons,
            apps,
            time_locale,
        }
    }
}

fn q(s: impl AsRef<str>) -> QString {
    QString::from(s.as_ref())
}

fn s(q: &QString) -> String {
    String::from(q)
}

impl SettingsRust {
    fn layer_kind(&self) -> Layer {
        if self.layer == 1 {
            Layer::Fn
        } else {
            Layer::Strip
        }
    }

    fn sel(&self) -> Option<usize> {
        usize::try_from(self.selected).ok()
    }

    fn lang(&self) -> Lang {
        Lang::from_setting(
            &self
                .model
                .section_str("general", "language")
                .unwrap_or_default(),
        )
    }
}

impl qobject::Settings {
    /// Re-check, re-draw and tell QML everything may have changed.
    fn refresh(mut self: Pin<&mut Self>) {
        let (picture, error, count, dirty, own, lang, selected) = {
            let mut this = self.as_mut().rust_mut();
            let this = &mut *this;
            let layer = this.layer_kind();
            let error = match this.model.validate() {
                Ok(cfg) => {
                    this.last_good = Some(cfg);
                    String::new()
                }
                Err(e) => e,
            };
            if let Some(cfg) = &this.last_good {
                let pins = this.model.pins().to_vec();
                this.shown = this
                    .painter
                    .paint(cfg, layer, &pins, this.time_locale.as_deref());
            }
            let count = this.model.count(layer) as i32;
            let selected = if this.selected >= count {
                -1
            } else {
                this.selected
            };
            let lang = match this.lang() {
                Lang::Italian => "it",
                Lang::English => "en",
            };
            (
                format!("data:image/png;base64,{}", base64(&this.shown.png)),
                error,
                count,
                this.model.is_dirty(),
                this.model.has_own_layout,
                lang,
                selected,
            )
        };
        self.as_mut().set_picture(q(picture));
        self.as_mut().set_error(q(error));
        self.as_mut().set_widget_count(count);
        self.as_mut().set_selected(selected);
        self.as_mut().set_dirty(dirty);
        self.as_mut().set_own_layout(own);
        self.as_mut().set_ui_lang(q(lang));
        let next = self.revision.wrapping_add(1);
        self.as_mut().set_revision(next);
    }

    fn tr(&self, key: &QString) -> QString {
        q(strings::tr(self.lang(), &s(key)))
    }

    fn start_view(&self) -> cxx_qt_lib::QList<i32> {
        let mut out = cxx_qt_lib::QList::default();
        if let Ok(show) = std::env::var("LISTELLO_SETTINGS_SHOW") {
            for n in show.split(',').filter_map(|n| n.trim().parse::<i32>().ok()) {
                out.append(n);
            }
        }
        out
    }

    fn set_strip_layer(mut self: Pin<&mut Self>, layer: i32) {
        if self.layer != layer {
            self.as_mut().set_layer(layer);
            self.as_mut().set_selected(-1);
        }
        self.refresh();
    }

    fn widget_at(&self, x: f64) -> i32 {
        self.shown
            .spans
            .iter()
            .chain(&self.shown.targets)
            .position(|(left, width)| x >= *left && x < left + width)
            .map_or(-1, |i| (i % self.shown.spans.len().max(1)) as i32)
    }

    fn span_x(&self, index: i32) -> f64 {
        span(&self.shown, index).0
    }

    fn span_width(&self, index: i32) -> f64 {
        span(&self.shown, index).1
    }

    /// Where a widget dropped at `x` goes: before the first widget whose middle is right of
    /// `x`.
    fn drop_slot(&self, x: f64) -> i32 {
        self.shown
            .spans
            .iter()
            .position(|(left, width)| x < left + width / 2.0)
            .unwrap_or(self.shown.spans.len()) as i32
    }

    /// The x of the gap before widget `slot` (or after the last), for the drop marker.
    fn slot_x(&self, slot: i32) -> f64 {
        let spans = &self.shown.spans;
        match usize::try_from(slot).ok() {
            Some(0) | None => spans.first().map_or(0.0, |s| s.0),
            Some(i) if i >= spans.len() => spans.last().map_or(0.0, |s| s.0 + s.1),
            Some(i) => (spans[i - 1].0 + spans[i - 1].1 + spans[i].0) / 2.0,
        }
    }

    fn kind(&self, index: i32) -> QString {
        let index = usize::try_from(index).unwrap_or(usize::MAX);
        q(self.model.kind(self.layer_kind(), index))
    }

    fn kind_count(&self) -> i32 {
        KINDS.len() as i32
    }

    fn kind_at(&self, i: i32) -> QString {
        q(KINDS.get(i as usize).copied().unwrap_or_default())
    }

    /// After the widget picked, or at the end; the new one is picked.
    fn add_widget(mut self: Pin<&mut Self>, kind: &QString) {
        let at = {
            let mut this = self.as_mut().rust_mut();
            let layer = this.layer_kind();
            let at = this.sel().map_or(usize::MAX, |i| i + 1);
            this.model.add(layer, &s(kind), at) as i32
        };
        self.as_mut().set_selected(at);
        self.refresh();
    }

    fn remove_selected(mut self: Pin<&mut Self>) {
        {
            let mut this = self.as_mut().rust_mut();
            let layer = this.layer_kind();
            if let Some(i) = this.sel() {
                this.model.remove(layer, i);
            }
        }
        self.as_mut().set_selected(-1);
        self.refresh();
    }

    /// Move widget `from` into the gap before `slot` (counted with it still in place).
    fn move_widget(mut self: Pin<&mut Self>, from: i32, slot: i32) {
        let (Ok(from), Ok(slot)) = (usize::try_from(from), usize::try_from(slot)) else {
            return;
        };
        let to = if slot > from { slot - 1 } else { slot };
        {
            let mut this = self.as_mut().rust_mut();
            let layer = this.layer_kind();
            this.model.move_to(layer, from, to);
        }
        self.as_mut().set_selected(to as i32);
        self.refresh();
    }

    fn get_str(&self, key: &QString) -> QString {
        let Some(i) = self.sel() else {
            return QString::default();
        };
        q(self
            .model
            .get_str(self.layer_kind(), i, &s(key))
            .unwrap_or_default())
    }

    fn set_str(mut self: Pin<&mut Self>, key: &QString, text: &QString) {
        self.as_mut()
            .edit(|m, layer, i| m.set_str(layer, i, &s(key), &s(text)));
    }

    fn get_bool(&self, key: &QString, fallback: bool) -> bool {
        self.sel()
            .and_then(|i| self.model.get_bool(self.layer_kind(), i, &s(key)))
            .unwrap_or(fallback)
    }

    fn set_bool(mut self: Pin<&mut Self>, key: &QString, on: bool) {
        self.as_mut()
            .edit(|m, layer, i| m.set_bool(layer, i, &s(key), on));
    }

    fn get_list(&self, key: &QString) -> QString {
        let Some(i) = self.sel() else {
            return QString::default();
        };
        q(self.model.get_list(self.layer_kind(), i, &s(key)))
    }

    fn set_list(mut self: Pin<&mut Self>, key: &QString, text: &QString) {
        self.as_mut()
            .edit(|m, layer, i| m.set_list(layer, i, &s(key), &s(text)));
    }

    fn sizing_mode(&self) -> QString {
        let Some(i) = self.sel() else {
            return q("auto");
        };
        q(self.model.sizing(self.layer_kind(), i).0)
    }

    fn sizing_amount(&self) -> f64 {
        self.sel()
            .map_or(0.0, |i| self.model.sizing(self.layer_kind(), i).1)
    }

    fn set_sizing(mut self: Pin<&mut Self>, mode: &QString, amount: f64) {
        self.as_mut()
            .edit(|m, layer, i| m.set_sizing(layer, i, &s(mode), amount));
    }

    fn item_count(&self, list: &QString) -> i32 {
        self.sel().map_or(0, |i| {
            self.model.item_count(self.layer_kind(), i, &s(list)) as i32
        })
    }

    fn item_str(&self, list: &QString, item: i32, key: &QString) -> QString {
        let (Some(i), Ok(item)) = (self.sel(), usize::try_from(item)) else {
            return QString::default();
        };
        q(self
            .model
            .item_str(self.layer_kind(), i, &s(list), item, &s(key)))
    }

    fn set_item_str(
        mut self: Pin<&mut Self>,
        list: &QString,
        item: i32,
        key: &QString,
        text: &QString,
    ) {
        let Ok(item) = usize::try_from(item) else {
            return;
        };
        self.as_mut()
            .edit(|m, layer, i| m.set_item_str(layer, i, &s(list), item, &s(key), &s(text)));
    }

    fn add_item(mut self: Pin<&mut Self>, list: &QString) {
        self.as_mut()
            .edit(|m, layer, i| m.add_item(layer, i, &s(list)));
    }

    fn remove_item(mut self: Pin<&mut Self>, list: &QString, item: i32) {
        let Ok(item) = usize::try_from(item) else {
            return;
        };
        self.as_mut()
            .edit(|m, layer, i| m.remove_item(layer, i, &s(list), item));
    }

    fn move_item(mut self: Pin<&mut Self>, list: &QString, from: i32, to: i32) {
        let (Ok(from), Ok(to)) = (usize::try_from(from), usize::try_from(to)) else {
            return;
        };
        self.as_mut()
            .edit(|m, layer, i| m.move_item(layer, i, &s(list), from, to));
    }

    fn icon_count(&self, context: &QString) -> i32 {
        let context = s(context);
        self.icons.iter().filter(|n| icon_fits(&context, n)).count() as i32
    }

    fn icon_name(&self, context: &QString, i: i32) -> QString {
        let context = s(context);
        q(self
            .icons
            .iter()
            .filter(|n| icon_fits(&context, n))
            .nth(i as usize)
            .cloned()
            .unwrap_or_default())
    }

    fn key_from_scan_code(&self, scan: i32) -> QString {
        q(keys::from_native_scan_code(scan).unwrap_or_default())
    }

    fn key_label(&self, names: &QString) -> QString {
        q(keys::label(&s(names), self.lang() == Lang::Italian))
    }

    fn special_key_count(&self) -> i32 {
        keys::SPECIAL.len() as i32
    }

    fn special_key(&self, i: i32) -> QString {
        q(keys::SPECIAL.get(i as usize).copied().unwrap_or_default())
    }

    /// `format` drawn with today's date, in the language the Touch Bar would use.
    fn format_example(&self, format: &QString) -> QString {
        let locale = match self.model.section_str("general", "language").as_deref() {
            Some("italian") => Some("it_IT".to_string()),
            Some("english") => Some("en_US".to_string()),
            _ => self.time_locale.clone(),
        }
        .and_then(|l| listello_ui::config::clock_locale(&l))
        .unwrap_or(chrono::Locale::POSIX);
        let format = s(format);
        let items: Vec<chrono::format::Item> =
            chrono::format::StrftimeItems::new_with_locale(&format, locale).collect();
        if items.contains(&chrono::format::Item::Error) {
            return QString::default();
        }
        q(chrono::Local::now()
            .format_localized_with_items(items.iter(), locale)
            .to_string())
    }

    fn icon_url(&self, name: &QString) -> QString {
        let name = s(name);
        if name.is_empty() {
            return QString::default();
        }
        let path = self.share.join(format!("{}.svg", name.replace('-', "_")));
        if path.is_file() {
            q(format!("file://{}", path.display()))
        } else {
            QString::default()
        }
    }

    fn pin_count(&self) -> i32 {
        self.model.pins().len() as i32
    }

    fn pin_name(&self, i: i32) -> QString {
        let Some(id) = self.model.pins().get(i as usize) else {
            return QString::default();
        };
        let name = self
            .painter
            .catalog()
            .find(id)
            .map_or_else(|| id.clone(), |e| e.name.clone());
        q(name)
    }

    fn pin_icon(mut self: Pin<&mut Self>, i: i32) -> QString {
        let mut this = self.as_mut().rust_mut();
        let Some(id) = this.model.pins().get(i as usize).cloned() else {
            return QString::default();
        };
        let png = this.painter.app_icon(&id).and_then(|icon| {
            let mut out = Vec::new();
            listello_surface::image_from_rgba(icon.width, icon.height, &icon.rgba)?
                .write_to_png(&mut out)
                .ok()?;
            Some(out)
        });
        png.map_or_else(QString::default, |png| {
            q(format!("data:image/png;base64,{}", base64(&png)))
        })
    }

    fn add_pin(mut self: Pin<&mut Self>, app: i32) {
        {
            let mut this = self.as_mut().rust_mut();
            let Some((id, _)) = this.apps.get(app as usize).cloned() else {
                return;
            };
            let mut pins = this.model.pins().to_vec();
            pins.push(id);
            this.model.set_pins(pins);
        }
        self.refresh();
    }

    fn remove_pin(mut self: Pin<&mut Self>, i: i32) {
        {
            let mut this = self.as_mut().rust_mut();
            let mut pins = this.model.pins().to_vec();
            if (i as usize) < pins.len() {
                pins.remove(i as usize);
            }
            this.model.set_pins(pins);
        }
        self.refresh();
    }

    fn move_pin(mut self: Pin<&mut Self>, from: i32, to: i32) {
        {
            let mut this = self.as_mut().rust_mut();
            let mut pins = this.model.pins().to_vec();
            let (from, to) = (from as usize, to as usize);
            if from < pins.len() && to < pins.len() {
                let p = pins.remove(from);
                pins.insert(to, p);
            }
            this.model.set_pins(pins);
        }
        self.refresh();
    }

    fn app_count(&self) -> i32 {
        self.apps.len() as i32
    }

    fn app_name(&self, i: i32) -> QString {
        q(self
            .apps
            .get(i as usize)
            .map(|(_, name)| name.clone())
            .unwrap_or_default())
    }

    fn section_str(&self, section: &QString, key: &QString) -> QString {
        q(self
            .model
            .section_str(&s(section), &s(key))
            .unwrap_or_default())
    }

    fn set_section_str(mut self: Pin<&mut Self>, section: &QString, key: &QString, text: &QString) {
        self.as_mut()
            .rust_mut()
            .model
            .set_section_str(&s(section), &s(key), &s(text));
        self.refresh();
    }

    fn section_num(&self, section: &QString, key: &QString, fallback: f64) -> f64 {
        self.model
            .section_f64(&s(section), &s(key))
            .unwrap_or(fallback)
    }

    /// A NaN removes the setting, so its default applies.
    fn set_section_num(mut self: Pin<&mut Self>, section: &QString, key: &QString, number: f64) {
        let number = (!number.is_nan()).then_some(number);
        self.as_mut()
            .rust_mut()
            .model
            .set_section_f64(&s(section), &s(key), number);
        self.refresh();
    }

    fn section_bool(&self, section: &QString, key: &QString) -> bool {
        self.model
            .section_bool(&s(section), &s(key))
            .unwrap_or(false)
    }

    fn set_section_bool(mut self: Pin<&mut Self>, section: &QString, key: &QString, on: bool) {
        self.as_mut()
            .rust_mut()
            .model
            .set_section_bool(&s(section), &s(key), on);
        self.refresh();
    }

    fn check_updates(&self) -> bool {
        self.model.check_updates()
    }

    fn set_check_updates(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().rust_mut().model.set_check_updates(on);
        self.refresh();
    }

    fn reload_listello(&self) {
        tray::reload_listello();
    }

    fn stop_listello(&self) {
        tray::stop_listello();
    }

    fn start_at_login(&self) -> bool {
        lifecycle::start_at_login()
    }

    fn set_start_at_login(&self, on: bool) {
        lifecycle::set_start_at_login(on);
    }

    fn changes_touch_bar(&self) -> bool {
        self.model.changes_touch_bar()
    }

    fn apply(mut self: Pin<&mut Self>) -> bool {
        let result = self.as_mut().rust_mut().model.apply();
        if let Err(e) = &result {
            eprintln!("listello-settings: could not apply: {e}");
        }
        // The agent watches the file and starts the tray when it is turned on; the tray watches
        // it too and leaves when it is turned off.
        self.refresh();
        result.is_ok()
    }

    fn cancel(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().model.cancel();
        self.refresh();
    }

    fn restore_default(mut self: Pin<&mut Self>) -> bool {
        let result = self.as_mut().rust_mut().model.restore_default();
        if let Err(e) = &result {
            eprintln!("listello-settings: could not restore the default: {e}");
        }
        self.as_mut().set_selected(-1);
        self.refresh();
        result.is_ok()
    }

    /// Change the widget picked, then refresh.
    fn edit(mut self: Pin<&mut Self>, change: impl FnOnce(&mut Model, Layer, usize)) {
        {
            let mut this = self.as_mut().rust_mut();
            let layer = this.layer_kind();
            if let Some(i) = this.sel() {
                change(&mut this.model, layer, i);
            }
        }
        self.refresh();
    }
}

/// Which packaged icons make sense where: folder icons for folders, and for buttons
/// everything but the pictures other widgets draw by themselves (battery, wifi, pins).
fn icon_fits(context: &str, name: &str) -> bool {
    let folderish = ["folder", "bin", "trash", "download", "screenshot"]
        .iter()
        .any(|p| name.starts_with(p));
    match context {
        "folder" => folderish,
        _ => {
            !folderish
                && !["battery", "network", "pin", "bolt"]
                    .iter()
                    .any(|p| name.starts_with(p))
        }
    }
}

fn span(picture: &Picture, index: i32) -> (f64, f64) {
    usize::try_from(index)
        .ok()
        .and_then(|i| picture.targets.get(i).copied())
        .unwrap_or((0.0, 0.0))
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
