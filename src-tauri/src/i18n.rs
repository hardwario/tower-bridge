//! Translations for texts produced by the backend: tray menu, notifications
//! and native dialogs. The dashboard has its own dictionaries (`src/lib/i18n`).
//!
//! Placeholders are written as `{name}` and filled with [`fill`].

use std::sync::OnceLock;

use serde::Serialize;
use tower_bridge_core::model::Metric;

use crate::settings::LanguagePref;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    En,
    Cs,
    Sk,
    De,
    Pl,
}

impl Lang {
    pub fn from_code(code: &str) -> Option<Lang> {
        let primary = code.split(['-', '_']).next()?.to_ascii_lowercase();
        Some(match primary.as_str() {
            "en" => Lang::En,
            "cs" => Lang::Cs,
            "sk" => Lang::Sk,
            "de" => Lang::De,
            "pl" => Lang::Pl,
            _ => return None,
        })
    }

    /// Supported language of the operating system (English otherwise).
    pub fn system() -> Lang {
        static SYSTEM: OnceLock<Lang> = OnceLock::new();
        *SYSTEM.get_or_init(|| {
            sys_locale::get_locales()
                .find_map(|l| Lang::from_code(&l))
                .unwrap_or(Lang::En)
        })
    }

    pub fn effective(pref: LanguagePref) -> Lang {
        match pref {
            LanguagePref::System => Lang::system(),
            LanguagePref::En => Lang::En,
            LanguagePref::Cs => Lang::Cs,
            LanguagePref::Sk => Lang::Sk,
            LanguagePref::De => Lang::De,
            LanguagePref::Pl => Lang::Pl,
        }
    }

    pub fn decimal_comma(self) -> bool {
        self != Lang::En
    }

    pub fn tr(self) -> &'static Tr {
        match self {
            Lang::En => &EN,
            Lang::Cs => &CS,
            Lang::Sk => &SK,
            Lang::De => &DE,
            Lang::Pl => &PL,
        }
    }
}

/// Language currently configured in the app.
pub fn current(app: &tauri::AppHandle) -> Lang {
    use tauri::Manager;
    app.try_state::<crate::state::AppState>()
        .map(|s| Lang::effective(s.settings().language))
        .unwrap_or_else(Lang::system)
}

/// Replace `{name}` placeholders.
pub fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

fn metric_index(m: Metric) -> usize {
    match m {
        Metric::Temperature => 0,
        Metric::Humidity => 1,
        Metric::Illuminance => 2,
        Metric::Pressure => 3,
        Metric::Co2 => 4,
        Metric::Voc => 5,
    }
}

#[derive(Debug)]
pub struct Tr {
    metrics: [&'static str; 6],
    high: [&'static str; 6],
    low: [&'static str; 6],
    pub back_to_normal: &'static str,
    pub above_limit: &'static str,
    pub below_limit: &'static str,
    pub within_limits: &'static str,
    pub open_dashboard: &'static str,
    pub open_at_login: &'static str,
    pub mqtt_error: &'static str,
    pub mqtt_connecting: &'static str,
    pub quit: &'static str,
    pub searching: &'static str,
    pub measuring: &'static str,
    pub updated: &'static str,
    pub not_connected: &'static str,
    pub no_permission: &'static str,
    pub i2c_dev_missing: &'static str,
    pub busy: &'static str,
    pub bridge_error: &'static str,
    pub no_module: &'static str,
    /// The tray status while several bridges are plugged in.
    pub several_bridges: &'static str,
    pub lost_title: &'static str,
    pub lost_body: &'static str,
    pub reconnected_title: &'static str,
    pub reconnected_body: &'static str,
    pub test_body: &'static str,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub webview2_missing: &'static str,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub download: &'static str,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub cancel: &'static str,
    pub export_title: &'static str,
    pub xlsx_sheet: &'static str,
    pub xlsx_time: &'static str,
    pub xlsx_device: &'static str,
    pub xlsx_channel: &'static str,
    pub altitude: &'static str,
    pub move_to_applications: &'static str,
    pub cannot_save_settings: &'static str,
}

impl Tr {
    pub fn metric(&self, m: Metric) -> &'static str {
        self.metrics[metric_index(m)]
    }

    pub fn high(&self, m: Metric) -> &'static str {
        self.high[metric_index(m)]
    }

    pub fn low(&self, m: Metric) -> &'static str {
        self.low[metric_index(m)]
    }
}

static EN: Tr = Tr {
    metrics: [
        "Temperature",
        "Humidity",
        "Illuminance",
        "Pressure",
        "CO₂",
        "VOC",
    ],
    high: [
        "High temperature",
        "High humidity",
        "High illuminance",
        "High pressure",
        "High CO₂",
        "High VOC",
    ],
    low: [
        "Low temperature",
        "Low humidity",
        "Low illuminance",
        "Low pressure",
        "Low CO₂",
        "Low VOC",
    ],
    back_to_normal: "{metric} is back to normal",
    above_limit: "{value} is above the {limit} limit.",
    below_limit: "{value} is below the {limit} limit.",
    within_limits: "{value} is within limits again.",
    open_dashboard: "Open Dashboard…",
    open_at_login: "Open at Login",
    mqtt_error: "error",
    mqtt_connecting: "connecting…",
    quit: "Quit TOWER Bridge",
    searching: "Searching for TOWER Bridge…",
    measuring: "Measuring…",
    updated: "Updated {time}",
    not_connected: "TOWER Bridge not connected",
    no_permission: "No permission to open the bridge (see help)",
    i2c_dev_missing: "Kernel module i2c-dev not loaded (see help)",
    busy: "Bridge is used by another program",
    bridge_error: "Bridge error, reconnecting…",
    no_module: "No sensors found",
    several_bridges: "⚠ More than one TOWER Bridge Module connected",
    lost_title: "TOWER Bridge disconnected",
    lost_body: "No measurements for a minute. Check the USB connection.",
    reconnected_title: "TOWER Bridge reconnected",
    reconnected_body: "Measurements have resumed.",
    test_body: "Notifications are working. You will be alerted when a value crosses a limit.",
    webview2_missing: "The dashboard needs the Microsoft Edge WebView2 Runtime, which is not installed.\n\n\
The tray icon keeps working. Download WebView2 now?",
    download: "Download",
    cancel: "Cancel",
    export_title: "Export readings",
    xlsx_sheet: "Readings",
    xlsx_time: "Time",
    xlsx_device: "Device",
    xlsx_channel: "I²C channel",
    altitude: "Altitude",
    move_to_applications: "Move TOWER Bridge to the Applications folder before enabling Open at Login.",
    cannot_save_settings: "Cannot save settings: {error}",
};

static CS: Tr = Tr {
    metrics: ["Teplota", "Vlhkost", "Osvětlení", "Tlak", "CO₂", "VOC"],
    high: [
        "Vysoká teplota",
        "Vysoká vlhkost",
        "Vysoké osvětlení",
        "Vysoký tlak",
        "Vysoké CO₂",
        "Vysoké VOC",
    ],
    low: [
        "Nízká teplota",
        "Nízká vlhkost",
        "Nízké osvětlení",
        "Nízký tlak",
        "Nízké CO₂",
        "Nízké VOC",
    ],
    back_to_normal: "{metric} je opět v normě",
    above_limit: "{value} je nad limitem {limit}.",
    below_limit: "{value} je pod limitem {limit}.",
    within_limits: "{value} je opět v povoleném rozsahu.",
    open_dashboard: "Otevřít přehled…",
    open_at_login: "Spouštět po přihlášení",
    mqtt_error: "chyba",
    mqtt_connecting: "připojování…",
    quit: "Ukončit TOWER Bridge",
    searching: "Hledám TOWER Bridge…",
    measuring: "Měřím…",
    updated: "Aktualizováno {time}",
    not_connected: "TOWER Bridge není připojen",
    no_permission: "Chybí oprávnění k zařízení (viz nápověda)",
    i2c_dev_missing: "Modul jádra i2c-dev není načten (viz nápověda)",
    busy: "Zařízení používá jiný program",
    bridge_error: "Chyba zařízení, připojuji znovu…",
    no_module: "Senzory nenalezeny",
    several_bridges: "⚠ Připojeno více TOWER Bridge Module",
    lost_title: "TOWER Bridge odpojen",
    lost_body: "Už minutu žádné měření. Zkontrolujte připojení USB.",
    reconnected_title: "TOWER Bridge znovu připojen",
    reconnected_body: "Měření pokračuje.",
    test_body: "Oznámení fungují. Upozorníme vás, když hodnota překročí limit.",
    webview2_missing: "Okno aplikace potřebuje Microsoft Edge WebView2 Runtime, který není nainstalován.\n\n\
Ikona v oznamovací oblasti funguje dál. Stáhnout WebView2 nyní?",
    download: "Stáhnout",
    cancel: "Zrušit",
    export_title: "Export měření",
    xlsx_sheet: "Měření",
    xlsx_time: "Čas",
    xlsx_device: "Zařízení",
    xlsx_channel: "Kanál I²C",
    altitude: "Nadmořská výška",
    move_to_applications: "Před zapnutím spouštění po přihlášení přesuňte TOWER Bridge do složky Aplikace.",
    cannot_save_settings: "Nastavení nelze uložit: {error}",
};

static SK: Tr = Tr {
    metrics: ["Teplota", "Vlhkosť", "Osvetlenie", "Tlak", "CO₂", "VOC"],
    high: [
        "Vysoká teplota",
        "Vysoká vlhkosť",
        "Vysoké osvetlenie",
        "Vysoký tlak",
        "Vysoké CO₂",
        "Vysoké VOC",
    ],
    low: [
        "Nízka teplota",
        "Nízka vlhkosť",
        "Nízke osvetlenie",
        "Nízky tlak",
        "Nízke CO₂",
        "Nízke VOC",
    ],
    back_to_normal: "{metric} je opäť v norme",
    above_limit: "{value} je nad limitom {limit}.",
    below_limit: "{value} je pod limitom {limit}.",
    within_limits: "{value} je opäť v povolenom rozsahu.",
    open_dashboard: "Otvoriť prehľad…",
    open_at_login: "Spúšťať po prihlásení",
    mqtt_error: "chyba",
    mqtt_connecting: "pripájanie…",
    quit: "Ukončiť TOWER Bridge",
    searching: "Hľadám TOWER Bridge…",
    measuring: "Meriam…",
    updated: "Aktualizované {time}",
    not_connected: "TOWER Bridge nie je pripojený",
    no_permission: "Chýba oprávnenie k zariadeniu (pozri pomocníka)",
    i2c_dev_missing: "Modul jadra i2c-dev nie je načítaný (pozri pomocníka)",
    busy: "Zariadenie používa iný program",
    bridge_error: "Chyba zariadenia, pripájam znova…",
    no_module: "Senzory sa nenašli",
    several_bridges: "⚠ Pripojených viac TOWER Bridge Module",
    lost_title: "TOWER Bridge odpojený",
    lost_body: "Už minútu žiadne meranie. Skontrolujte pripojenie USB.",
    reconnected_title: "TOWER Bridge znova pripojený",
    reconnected_body: "Meranie pokračuje.",
    test_body: "Oznámenia fungujú. Upozorníme vás, keď hodnota prekročí limit.",
    webview2_missing: "Okno aplikácie potrebuje Microsoft Edge WebView2 Runtime, ktorý nie je nainštalovaný.\n\n\
Ikona v oblasti oznámení funguje ďalej. Stiahnuť WebView2 teraz?",
    download: "Stiahnuť",
    cancel: "Zrušiť",
    export_title: "Export meraní",
    xlsx_sheet: "Merania",
    xlsx_time: "Čas",
    xlsx_device: "Zariadenie",
    xlsx_channel: "Kanál I²C",
    altitude: "Nadmorská výška",
    move_to_applications: "Pred zapnutím spúšťania po prihlásení presuňte TOWER Bridge do priečinka Aplikácie.",
    cannot_save_settings: "Nastavenia nie je možné uložiť: {error}",
};

static DE: Tr = Tr {
    metrics: [
        "Temperatur",
        "Luftfeuchtigkeit",
        "Beleuchtungsstärke",
        "Luftdruck",
        "CO₂",
        "VOC",
    ],
    high: [
        "Hohe Temperatur",
        "Hohe Luftfeuchtigkeit",
        "Hohe Beleuchtungsstärke",
        "Hoher Luftdruck",
        "Hoher CO₂-Wert",
        "Hoher VOC-Wert",
    ],
    low: [
        "Niedrige Temperatur",
        "Niedrige Luftfeuchtigkeit",
        "Niedrige Beleuchtungsstärke",
        "Niedriger Luftdruck",
        "Niedriger CO₂-Wert",
        "Niedriger VOC-Wert",
    ],
    back_to_normal: "{metric} wieder normal",
    above_limit: "{value} liegt über dem Grenzwert von {limit}.",
    below_limit: "{value} liegt unter dem Grenzwert von {limit}.",
    within_limits: "{value} liegt wieder im zulässigen Bereich.",
    open_dashboard: "Dashboard öffnen…",
    open_at_login: "Bei Anmeldung öffnen",
    mqtt_error: "Fehler",
    mqtt_connecting: "verbinde…",
    quit: "TOWER Bridge beenden",
    searching: "Suche TOWER Bridge…",
    measuring: "Messung läuft…",
    updated: "Aktualisiert {time}",
    not_connected: "TOWER Bridge nicht verbunden",
    no_permission: "Keine Berechtigung für das Gerät (siehe Hilfe)",
    i2c_dev_missing: "Kernelmodul i2c-dev nicht geladen (siehe Hilfe)",
    busy: "Gerät wird von einem anderen Programm verwendet",
    bridge_error: "Gerätefehler, neue Verbindung…",
    no_module: "Keine Sensoren gefunden",
    several_bridges: "⚠ Mehr als ein TOWER Bridge Module angeschlossen",
    lost_title: "TOWER Bridge getrennt",
    lost_body: "Seit einer Minute keine Messwerte. Bitte die USB-Verbindung prüfen.",
    reconnected_title: "TOWER Bridge wieder verbunden",
    reconnected_body: "Die Messung läuft wieder.",
    test_body: "Benachrichtigungen funktionieren. Sie werden informiert, wenn ein Wert einen Grenzwert überschreitet.",
    webview2_missing: "Das Dashboard benötigt die Microsoft Edge WebView2 Runtime, die nicht installiert ist.\n\n\
Das Symbol im Infobereich funktioniert weiterhin. WebView2 jetzt herunterladen?",
    download: "Herunterladen",
    cancel: "Abbrechen",
    export_title: "Messwerte exportieren",
    xlsx_sheet: "Messwerte",
    xlsx_time: "Zeit",
    xlsx_device: "Gerät",
    xlsx_channel: "I²C-Kanal",
    altitude: "Höhe",
    move_to_applications: "Verschieben Sie TOWER Bridge in den Ordner „Programme“, bevor Sie „Bei Anmeldung öffnen“ aktivieren.",
    cannot_save_settings: "Einstellungen können nicht gespeichert werden: {error}",
};

static PL: Tr = Tr {
    metrics: [
        "Temperatura",
        "Wilgotność",
        "Oświetlenie",
        "Ciśnienie",
        "CO₂",
        "VOC",
    ],
    high: [
        "Wysoka temperatura",
        "Wysoka wilgotność",
        "Wysokie natężenie oświetlenia",
        "Wysokie ciśnienie",
        "Wysokie CO₂",
        "Wysokie VOC",
    ],
    low: [
        "Niska temperatura",
        "Niska wilgotność",
        "Niskie natężenie oświetlenia",
        "Niskie ciśnienie",
        "Niskie CO₂",
        "Niskie VOC",
    ],
    back_to_normal: "{metric}: ponownie w normie",
    above_limit: "{value} przekracza limit {limit}.",
    below_limit: "{value} jest poniżej limitu {limit}.",
    within_limits: "{value} ponownie mieści się w zakresie.",
    open_dashboard: "Otwórz panel…",
    open_at_login: "Uruchamiaj przy logowaniu",
    mqtt_error: "błąd",
    mqtt_connecting: "łączenie…",
    quit: "Zakończ TOWER Bridge",
    searching: "Wyszukiwanie TOWER Bridge…",
    measuring: "Pomiar…",
    updated: "Zaktualizowano {time}",
    not_connected: "TOWER Bridge nie jest podłączony",
    no_permission: "Brak uprawnień do urządzenia (zobacz pomoc)",
    i2c_dev_missing: "Moduł jądra i2c-dev nie jest załadowany (zobacz pomoc)",
    busy: "Urządzenie jest używane przez inny program",
    bridge_error: "Błąd urządzenia, ponowne łączenie…",
    no_module: "Nie znaleziono czujników",
    several_bridges: "⚠ Podłączono więcej niż jeden TOWER Bridge Module",
    lost_title: "TOWER Bridge odłączony",
    lost_body: "Brak pomiarów od minuty. Sprawdź połączenie USB.",
    reconnected_title: "TOWER Bridge ponownie podłączony",
    reconnected_body: "Pomiary zostały wznowione.",
    test_body: "Powiadomienia działają. Otrzymasz alert, gdy wartość przekroczy limit.",
    webview2_missing: "Panel wymaga środowiska Microsoft Edge WebView2 Runtime, które nie jest zainstalowane.\n\n\
Ikona w zasobniku działa dalej. Pobrać WebView2 teraz?",
    download: "Pobierz",
    cancel: "Anuluj",
    export_title: "Eksport pomiarów",
    xlsx_sheet: "Pomiary",
    xlsx_time: "Czas",
    xlsx_device: "Urządzenie",
    xlsx_channel: "Kanał I²C",
    altitude: "Wysokość",
    move_to_applications: "Przed włączeniem uruchamiania przy logowaniu przenieś TOWER Bridge do folderu Aplikacje.",
    cannot_save_settings: "Nie można zapisać ustawień: {error}",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes() {
        assert_eq!(Lang::from_code("cs-CZ"), Some(Lang::Cs));
        assert_eq!(Lang::from_code("de_AT"), Some(Lang::De));
        assert_eq!(Lang::from_code("pl"), Some(Lang::Pl));
        assert_eq!(Lang::from_code("fr-FR"), None);
        assert_eq!(Lang::effective(LanguagePref::Sk), Lang::Sk);
    }

    /// Every text of a dictionary, by field (arrays get an index).
    fn texts(tr: &Tr) -> Vec<(String, String)> {
        // The Debug form lists the fields in order: name: "text" or [..].
        let debug = format!("{tr:#?}");
        let mut out = Vec::new();
        let mut field = String::new();
        let mut index = 0;
        for line in debug.lines().map(str::trim) {
            if let Some((name, rest)) = line.split_once(": ") {
                field = name.to_string();
                index = 0;
                if rest.starts_with('"') {
                    out.push((field.clone(), rest.trim_end_matches(',').to_string()));
                }
            } else if line.starts_with('"') {
                out.push((
                    format!("{field}[{index}]"),
                    line.trim_end_matches(',').to_string(),
                ));
                index += 1;
            }
        }
        out
    }

    fn placeholders(text: &str) -> Vec<String> {
        let mut found: Vec<String> = text
            .split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(name, _)| name.to_string()))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn every_language_has_every_text_with_the_same_placeholders() {
        let reference = texts(Lang::En.tr());
        assert!(reference.len() > 40, "{} texts", reference.len());
        for lang in [Lang::Cs, Lang::Sk, Lang::De, Lang::Pl] {
            let other = texts(lang.tr());
            assert_eq!(other.len(), reference.len(), "{lang:?}");
            for ((field, en), (f2, text)) in reference.iter().zip(&other) {
                assert_eq!(field, f2);
                assert!(text.len() > 2, "{lang:?} {field} is empty");
                assert_eq!(
                    placeholders(text),
                    placeholders(en),
                    "{lang:?} {field}: {text}"
                );
            }
        }
    }

    #[test]
    fn fill_placeholders() {
        let t = Lang::Cs.tr();
        assert_eq!(
            fill(t.above_limit, &[("value", "30,1 °C"), ("limit", "28,0 °C")]),
            "30,1 °C je nad limitem 28,0 °C."
        );
        assert_eq!(
            fill(t.back_to_normal, &[("metric", t.metric(Metric::Pressure))]),
            "Tlak je opět v normě"
        );
    }
}
