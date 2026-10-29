// SPDX-License-Identifier: GPL-3.0-only

//! The applet's words: Hungarian when the system speaks it, else English.

use std::sync::LazyLock;

pub struct Strings {
    pub wifi: &'static str,
    pub wired: &'static str,
    pub bluetooth: &'static str,
    pub vpn: &'static str,
    pub power: &'static str,
    pub dnd: &'static str,
    pub dark: &'static str,
    pub airplane: &'static str,
    pub mic: &'static str,
    pub on: &'static str,
    pub off: &'static str,
    pub not_connected: &'static str,
    pub connecting: &'static str,
    pub connected: &'static str,
    pub muted: &'static str,
    pub more_settings: &'static str,
    pub sound_output: &'static str,
    pub no_events: &'static str,
    pub no_calendar: &'static str,
    pub add_accounts: &'static str,
    pub all_day: &'static str,
    pub recording: &'static str,
    pub filming: &'static str,
    pub password_for: &'static str,
    pub password: &'static str,
    pub remember: &'static str,
    pub connect: &'static str,
    pub cancel: &'static str,
    pub edit: &'static str,
    pub layout: &'static str,
    pub dropdown: &'static str,
    pub side: &'static str,
    pub fullscreen: &'static str,
    pub only_active: &'static str,
    pub show_calendar: &'static str,
    pub show_clock: &'static str,
    pub show_all: &'static str,
    pub show_less: &'static str,
    pub performance: &'static str,
    pub balanced: &'static str,
    pub power_saver: &'static str,
    pub keep_awake: &'static str,
    pub tiling: &'static str,
    pub magnifier: &'static str,
    pub invert_colors: &'static str,
    pub screen_reader: &'static str,
    pub high_contrast: &'static str,
    pub screenshot: &'static str,
    pub workspaces: &'static str,
    pub touchpad: &'static str,
    pub add: &'static str,
    pub reset: &'static str,
    pub edit_hint: &'static str,
    pub panel_tiles: &'static str,
    pub panel_spacing: &'static str,
    pub small: &'static str,
    pub medium: &'static str,
    pub large: &'static str,
    pub months: [&'static str; 12],
    pub weekdays: [&'static str; 7],
}

const EN: Strings = Strings {
    wifi: "Wi-Fi",
    wired: "Wired",
    bluetooth: "Bluetooth",
    vpn: "VPN",
    power: "Power mode",
    dnd: "Do not disturb",
    dark: "Dark style",
    airplane: "Airplane mode",
    mic: "Microphone",
    on: "On",
    off: "Off",
    not_connected: "Not connected",
    connecting: "Connecting…",
    connected: "Connected",
    muted: "Muted",
    more_settings: "More settings",
    sound_output: "Sound output",
    no_events: "No events",
    no_calendar: "No synced calendars",
    add_accounts: "Add accounts",
    all_day: "All day",
    recording: "using the microphone",
    filming: "using the camera",
    password_for: "Password for",
    password: "Password",
    remember: "Remember",
    connect: "Connect",
    cancel: "Cancel",
    edit: "Edit",
    layout: "Opens as",
    dropdown: "Dropdown",
    side: "Side panel",
    fullscreen: "Fullscreen",
    only_active: "Show only active tiles",
    show_calendar: "Show calendar",
    show_clock: "Show clock on the panel",
    show_all: "All tiles",
    show_less: "Only active",
    keep_awake: "Keep awake",
    tiling: "Tiling",
    magnifier: "Magnifier",
    invert_colors: "Invert colors",
    screen_reader: "Screen reader",
    high_contrast: "High contrast",
    screenshot: "Screenshot",
    workspaces: "Workspaces",
    touchpad: "Touchpad",
    add: "Add",
    reset: "Reset tiles",
    edit_hint: "Drag tiles to reorder them; click one to resize it",
    panel_tiles: "Show active tiles on the panel",
    panel_spacing: "Space between the panel icons",
    small: "Small",
    medium: "Medium",
    large: "Large",
    performance: "Performance",
    balanced: "Balanced",
    power_saver: "Power saver",
    months: [
        "January", "February", "March", "April", "May", "June", "July", "August", "September",
        "October", "November", "December",
    ],
    weekdays: ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
};

const HU: Strings = Strings {
    wifi: "Wi-Fi",
    wired: "Vezetékes",
    bluetooth: "Bluetooth",
    vpn: "VPN",
    power: "Energiamód",
    dnd: "Ne zavarjanak",
    dark: "Sötét stílus",
    airplane: "Repülőgép mód",
    mic: "Mikrofon",
    on: "Be",
    off: "Ki",
    not_connected: "Nincs kapcsolat",
    connecting: "Csatlakozás…",
    connected: "Csatlakoztatva",
    muted: "Némítva",
    more_settings: "További beállítások",
    sound_output: "Hangkimenet",
    no_events: "Nincs esemény",
    no_calendar: "Nincs szinkronizált naptár",
    add_accounts: "Fiókok hozzáadása",
    all_day: "Egész nap",
    recording: "használja a mikrofont",
    filming: "használja a kamerát",
    password_for: "Jelszó:",
    password: "Jelszó",
    remember: "Megjegyzés",
    connect: "Csatlakozás",
    cancel: "Mégse",
    edit: "Szerkesztés",
    layout: "Megjelenés",
    dropdown: "Lenyíló",
    side: "Oldalsáv",
    fullscreen: "Teljes képernyő",
    only_active: "Csak az aktív csempék",
    show_calendar: "Naptár mutatása",
    show_clock: "Óra a panelen",
    show_all: "Összes csempe",
    show_less: "Csak az aktívak",
    keep_awake: "Ébren tartás",
    tiling: "Csempézés",
    magnifier: "Nagyító",
    invert_colors: "Színek invertálása",
    screen_reader: "Képernyőolvasó",
    high_contrast: "Nagy kontraszt",
    screenshot: "Képernyőkép",
    workspaces: "Munkaterületek",
    touchpad: "Érintőpad",
    add: "Hozzáadás",
    reset: "Csempék visszaállítása",
    edit_hint: "Húzd a csempéket az átrendezéshez; kattints egyre az átméretezéshez",
    panel_tiles: "Aktív csempék a panelen",
    panel_spacing: "Térköz a panel ikonjai között",
    small: "Kicsi",
    medium: "Közepes",
    large: "Nagy",
    performance: "Teljesítmény",
    balanced: "Kiegyensúlyozott",
    power_saver: "Energiatakarékos",
    months: [
        "január", "február", "március", "április", "május", "június", "július", "augusztus",
        "szeptember", "október", "november", "december",
    ],
    weekdays: ["hétfő", "kedd", "szerda", "csütörtök", "péntek", "szombat", "vasárnap"],
};

fn locale(keys: [&str; 3]) -> String {
    keys.iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default()
}

/// The words, in the language (LC_MESSAGES).
pub static T: LazyLock<&'static Strings> =
    LazyLock::new(|| if locale(["LC_ALL", "LC_MESSAGES", "LANG"]).starts_with("hu") { &HU } else { &EN });

/// Dates follow the region (LC_TIME), which may differ from the language:
/// English words with Hungarian dates is common.
static REGION: LazyLock<String> = LazyLock::new(|| locale(["LC_ALL", "LC_TIME", "LANG"]));

fn hungarian() -> bool {
    REGION.starts_with("hu")
}

/// The US: 12-hour clock, weeks from Sunday, month before day.
fn us() -> bool {
    REGION.starts_with("en_US")
}

fn names() -> &'static Strings {
    if hungarian() { &HU } else { &EN }
}

fn month(date: jiff::civil::Date) -> &'static str {
    names().months[date.month() as usize - 1]
}

fn weekday(date: jiff::civil::Date) -> &'static str {
    names().weekdays[date.weekday().to_monday_zero_offset() as usize]
}

/// Days before `date` in its week.
pub fn week_offset(date: jiff::civil::Date) -> i64 {
    if us() { date.weekday().to_sunday_zero_offset() as i64 } else { date.weekday().to_monday_zero_offset() as i64 }
}

/// The calendar's column heads, from the first day of the week.
pub fn weekday_heads() -> Vec<String> {
    let mut days: Vec<String> = if hungarian() {
        ["H", "K", "Sze", "Cs", "P", "Szo", "V"].map(String::from).to_vec()
    } else {
        EN.weekdays.iter().map(|d| d.chars().take(2).collect()).collect()
    };
    if us() {
        days.rotate_right(1);
    }
    days
}

/// "2026. október" or "October 2026"
pub fn month_title(date: jiff::civil::Date) -> String {
    if hungarian() { format!("{}. {}", date.year(), month(date)) } else { format!("{} {}", month(date), date.year()) }
}

/// "október 29., csütörtök", "Thursday, October 29" or "Thursday 29 October"
pub fn long_date(date: jiff::civil::Date) -> String {
    let (m, w, d) = (month(date), weekday(date), date.day());
    if hungarian() {
        format!("{m} {d}., {w}")
    } else if us() {
        format!("{w}, {m} {d}")
    } else {
        format!("{w} {d} {m}")
    }
}

/// "okt. 29.", "Oct 29" or "29 Oct"
pub fn short_date(date: jiff::civil::Date) -> String {
    let m: String = month(date).chars().take(3).collect();
    let d = date.day();
    if hungarian() {
        format!("{m}. {d}.")
    } else if us() {
        format!("{m} {d}")
    } else {
        format!("{d} {m}")
    }
}

/// "14:05", or "2:05 PM" in the US.
pub fn clock(time: jiff::civil::Time) -> String {
    time.strftime(if us() { "%-I:%M %p" } else { "%H:%M" }).to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates_follow_region() {
        // The test runs in whatever locale the machine has: check only shapes.
        let date = jiff::civil::date(2026, 10, 29);
        assert!(super::long_date(date).contains("29"));
        assert_eq!(super::weekday_heads().len(), 7);
        let offset = super::week_offset(date);
        assert!(offset == 3 || offset == 4);
    }
}
