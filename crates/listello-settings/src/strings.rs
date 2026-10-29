//! The window's words, in the two languages it speaks (D8 amendment). A key with no entry is
//! shown as itself, so a missing one is visible rather than blank.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    English,
    Italian,
}

impl Lang {
    /// `[general] language` as the window applies it to itself: "system" is the display
    /// language (`LC_ALL`, else `LC_MESSAGES`, else `LANG`), not the time format.
    pub fn from_setting(setting: &str) -> Lang {
        match setting {
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
}

pub fn tr(lang: Lang, key: &str) -> String {
    let pair = STRINGS.iter().find(|(k, _, _)| *k == key);
    match (pair, lang) {
        (Some((_, en, _)), Lang::English) => en.to_string(),
        (Some((_, _, it)), Lang::Italian) => it.to_string(),
        (None, _) => key.to_string(),
    }
}

/// (key, English, Italian)
const STRINGS: &[(&str, &str, &str)] = &[
    ("window_title", "listello", "listello"),
    ("strip", "Strip", "Barra"),
    ("fn_keys", "Fn keys", "Tasti Fn"),
    ("general", "General", "Generale"),
    ("apply", "Apply", "Applica"),
    ("cancel", "Cancel", "Annulla"),
    ("restore", "Restore the default", "Ripristina il predefinito"),
    (
        "restore_question",
        "Drop your own layout and pinned apps, and go back to the machine's?",
        "Eliminare il tuo layout e le app fissate, e tornare a quelli della macchina?",
    ),
    ("restore_confirm", "Restore", "Ripristina"),
    ("applied", "Sent to the Touch Bar.", "Inviato alla Touch Bar."),
    (
        "machine_layout",
        "You are seeing the machine's layout. Apply makes it your own.",
        "Stai vedendo il layout della macchina. Applica lo rende tuo.",
    ),
    (
        "invalid",
        "The Touch Bar would refuse this layout:",
        "La Touch Bar rifiuterebbe questo layout:",
    ),
    (
        "pick_hint",
        "Click a widget to change it, drag it to move it.",
        "Clicca un widget per modificarlo, trascinalo per spostarlo.",
    ),
    (
        "fn_empty",
        "Holding Fn changes nothing. Add widgets to show them while Fn is held.",
        "Tenere premuto Fn non cambia nulla. Aggiungi dei widget per mostrarli mentre Fn è premuto.",
    ),
    ("add", "Add", "Aggiungi"),
    ("remove", "Remove", "Rimuovi"),
    ("move_left", "Move left", "Sposta a sinistra"),
    ("move_right", "Move right", "Sposta a destra"),
    ("move_up", "Move up", "Sposta su"),
    ("move_down", "Move down", "Sposta giù"),
    // Widget kinds.
    ("kind_dock", "Dock", "Dock"),
    ("kind_folders", "Folders", "Cartelle"),
    ("kind_control_center", "Control centre", "Centro di controllo"),
    ("kind_network", "Wi-Fi", "Wi-Fi"),
    ("kind_battery", "Battery", "Batteria"),
    ("kind_clock", "Date and time", "Data e ora"),
    ("kind_button", "Button", "Pulsante"),
    ("kind_separator", "Separator", "Separatore"),
    // Width.
    ("width", "Width:", "Larghezza:"),
    ("width_auto", "Automatic", "Automatica"),
    ("width_fixed", "Fixed, in pixels", "Fissa, in pixel"),
    ("width_share", "Share of the free space", "Parte dello spazio libero"),
    // Button.
    ("label", "Label:", "Etichetta:"),
    ("icon", "Icon:", "Icona:"),
    ("no_icon", "None", "Nessuna"),
    ("keys", "Keys:", "Tasti:"),
    ("no_keys", "Choose keys…", "Scegli i tasti…"),
    ("press_keys", "Press the keys…", "Premi i tasti…"),
    (
        "record_hint",
        "Click, then press a key or a combination such as Ctrl + C.",
        "Clicca, poi premi un tasto o una combinazione come Ctrl + C.",
    ),
    ("special_key", "Or a special key…", "Oppure un tasto speciale…"),
    // Clock.
    ("format", "Shows:", "Mostra:"),
    ("format_custom", "Format (strftime):", "Formato (strftime):"),
    ("align", "Position:", "Posizione:"),
    ("align_start", "Left", "A sinistra"),
    ("align_center", "Centre", "Al centro"),
    ("align_end", "Right", "A destra"),
    // Dock.
    ("pinned", "Pinned apps:", "App fissate:"),
    ("add_app", "Pin an app…", "Fissa un'app…"),
    (
        "pinned_hint",
        "The same list the Touch Bar's own pin button changes.",
        "La stessa lista che cambia il pulsante di fissaggio sulla Touch Bar.",
    ),
    // Battery, network.
    ("show_percentage", "Show the percentage", "Mostra la percentuale"),
    ("show_bar", "Show the bar", "Mostra la barra"),
    ("show_ssid", "Show the network name", "Mostra il nome della rete"),
    // Control centre.
    ("buttons", "Buttons:", "Pulsanti:"),
    ("add_button", "Add a button", "Aggiungi un pulsante"),
    ("action", "Does:", "Fa:"),
    ("action_keys", "Presses keys", "Preme dei tasti"),
    ("action_lock", "Locks the screen", "Blocca lo schermo"),
    // Folders.
    ("folders", "Folders:", "Cartelle:"),
    ("add_folder", "Add a folder", "Aggiungi una cartella"),
    ("path", "Opens:", "Apre:"),
    ("same_when_empty", "Same icon when empty", "Stessa icona da vuota"),
    // General.
    ("language", "Language:", "Lingua:"),
    ("language_system", "Follow the system", "Segui il sistema"),
    (
        "language_hint",
        "For this window and the day and month names on the Touch Bar.",
        "Per questa finestra e per i nomi di giorni e mesi sulla Touch Bar.",
    ),
    ("previews", "Window pictures:", "Immagini delle finestre:"),
    ("previews_off", "Off", "Disattivate"),
    ("previews_snapshots", "Still pictures", "Immagini fisse"),
    ("previews_live", "Live", "Dal vivo"),
    ("preview_style", "Picture style:", "Stile dell'immagine:"),
    ("style_fill", "Fill the entry", "Riempie la voce"),
    ("style_whole", "Whole window, small", "Finestra intera, piccola"),
    ("style_stretch", "Whole window, stretched", "Finestra intera, stirata"),
    ("hold", "Hold to show window buttons:", "Tieni premuto per i pulsanti finestra:"),
    ("controls", "Window buttons stay for:", "I pulsanti finestra restano per:"),
    ("seconds", "seconds", "secondi"),
    ("gap", "Space between widgets:", "Spazio tra i widget:"),
    ("pixels", "pixels", "pixel"),
    ("tray", "System tray:", "Vassoio di sistema:"),
    ("tray_show", "Show the listello icon", "Mostra l'icona di listello"),
    ("updates", "Updates:", "Aggiornamenti:"),
    ("updates_check", "Check for updates", "Cerca aggiornamenti"),
    (
        "updates_hint",
        "About once a day listello asks GitHub whether a newer version exists and tells you. It never downloads or installs anything.",
        "Circa una volta al giorno listello chiede a GitHub se esiste una versione più recente e te lo dice. Non scarica né installa mai nulla.",
    ),
    ("reload", "Reload listello", "Ricarica listello"),
    (
        "reload_hint",
        "Makes the Touch Bar start over, for when it misbehaves. Right away, no Apply needed.",
        "Fa ripartire la Touch Bar da capo, per quando non si comporta bene. Subito, senza Applica.",
    ),
    ("stop", "Stop listello", "Ferma listello"),
    (
        "stop_hint",
        "Stops listello completely: this window, the tray icon and the Touch Bar program. The Touch Bar goes back to the keys the Mac draws itself. To start again, open listello from the application menu.",
        "Ferma listello del tutto: questa finestra, l'icona nel vassoio e il programma della Touch Bar. La Touch Bar torna ai tasti che disegna il Mac. Per riavviare, apri listello dal menu delle applicazioni.",
    ),
    ("login", "listello:", "listello:"),
    ("login_start", "Start at login", "Avvia all'accesso"),
    (
        "login_hint",
        "Right away, no Apply needed. Without it, listello starts only when you open it from the application menu.",
        "Subito, senza Applica. Senza, listello parte solo quando lo apri dal menu delle applicazioni.",
    ),
    // The question asked the first time a user ever starts listello.
    (
        "first_question",
        "Start listello every time you log in?",
        "Avviare listello ogni volta che accedi?",
    ),
    ("first_yes", "Yes, every time", "Sì, ogni volta"),
    ("first_no", "Only now", "Solo adesso"),
    // The tray's menu.
    ("tray_open", "Open settings", "Apri impostazioni"),
    ("tray_updates", "Check for updates", "Cerca aggiornamenti"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_none_is_blank() {
        for (i, (k, en, it)) in STRINGS.iter().enumerate() {
            assert!(!en.is_empty() && !it.is_empty(), "{k}");
            assert!(STRINGS[i + 1..].iter().all(|(o, _, _)| o != k), "{k} twice");
        }
        assert_eq!(tr(Lang::Italian, "apply"), "Applica");
        assert_eq!(tr(Lang::English, "nonsense"), "nonsense");
    }
}
