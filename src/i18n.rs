// SPDX-License-Identifier: GPL-3.0-only

//! The applet's words, in the interface language (Fluent files under
//! `i18n/`, English where a translation lacks one), and dates in the region's
//! format (ICU, as COSMIC's own clock formats them).

use std::sync::LazyLock;

use i18n_embed::{
    DesktopLanguageRequester,
    fluent::{FluentLanguageLoader, fluent_language_loader},
};
use icu::{
    calendar::{Gregorian, week::WeekInformation},
    datetime::{
        DateTimeFormatter, DateTimeFormatterPreferences, fieldsets,
        input::{Date as IcuDate, DateTime, Time},
        options::TimePrecision,
    },
    locale::Locale,
};

#[derive(rust_embed::RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

static LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();
    // No invisible direction marks around the app names put into a message.
    loader.set_use_isolating(false);
    let requested = DesktopLanguageRequester::requested_languages();
    if let Err(err) = i18n_embed::select(&loader, &Localizations, &requested) {
        tracing::warn!("cannot load translations: {err}");
    }
    loader
});

/// The words, as fields: `T.wifi`. Fluent ids are the field names with
/// dashes for underscores.
macro_rules! strings {
    ($($field:ident),* $(,)?) => {
        pub struct Strings {
            $(pub $field: &'static str,)*
        }

        fn load(loader: &FluentLanguageLoader) -> Strings {
            // Loaded once, kept for the applet's life.
            let text = |id: &str| -> &'static str { Box::leak(loader.get(id).into_boxed_str()) };
            Strings { $($field: text(&stringify!($field).replace('_', "-")),)* }
        }

        #[cfg(test)]
        const KEYS: &[&str] = &[$(stringify!($field)),*];
    };
}

strings!(
    wifi,
    wired,
    bluetooth,
    vpn,
    power,
    dnd,
    dark,
    airplane,
    mic,
    on,
    off,
    not_connected,
    connecting,
    connected,
    muted,
    more_settings,
    sound_output,
    no_events,
    no_calendar,
    add_accounts,
    all_day,
    password_for,
    password,
    remember,
    connect,
    cancel,
    edit,
    layout,
    dropdown,
    side,
    fullscreen,
    show_calendar,
    show_clock,
    performance,
    balanced,
    power_saver,
    keep_awake,
    tiling,
    magnifier,
    invert_colors,
    screen_reader,
    high_contrast,
    screenshot,
    workspaces,
    touchpad,
    add,
    reset,
    edit_hint,
    panel_icons,
    islands,
    panel_spacing,
    small,
    medium,
    large,
    icons_none,
    icons_active,
    icons_all,
    hotspot,
    network_name,
    hotspot_hint,
    updates,
    applications,
    no_apps_playing,
    pomodoro,
    focus,
    short_break,
    long_break,
    show_on_panel,
    pomodoro_break_time,
    pomodoro_focus_again,
);

pub static T: LazyLock<Strings> = LazyLock::new(|| load(&LOADER));

/// Messages that take the names of apps, placed where the language puts them.
const WITH_APPS: [&str; 2] = ["recording", "filming"];

/// "Discord, Zoom using the microphone": `what` is "recording" or "filming".
pub fn using(what: &str, apps: &[String]) -> String {
    debug_assert!(WITH_APPS.contains(&what));
    LOADER.get_args(what, std::collections::HashMap::from([("apps", apps.join(", "))]))
}

// ---------------------------------------------------------------- dates

/// The region's locale: LC_TIME may differ from the language, as English
/// words with Hungarian dates.
static LOCALE: LazyLock<Locale> = LazyLock::new(|| {
    ["LC_ALL", "LC_TIME", "LANG"]
        .iter()
        .find_map(|var| parse(&std::env::var(var).ok()?))
        .unwrap_or_else(|| Locale::try_from_str("en-US").expect("a valid locale"))
});

/// "hu_HU.UTF-8" → hu-HU; None for C, POSIX or nothing.
fn parse(value: &str) -> Option<Locale> {
    let tag = value.split(['.', '@']).next()?.replace('_', "-");
    if tag.is_empty() || tag == "C" || tag == "POSIX" {
        return None;
    }
    Locale::try_from_str(&tag).ok()
}

fn prefs() -> DateTimeFormatterPreferences {
    (&*LOCALE).into()
}

fn datetime(date: jiff::civil::Date, time: jiff::civil::Time) -> Option<DateTime<Gregorian>> {
    Some(DateTime {
        date: IcuDate::try_new_gregorian(date.year() as i32, date.month() as u8, date.day() as u8).ok()?,
        time: Time::try_new(time.hour() as u8, time.minute() as u8, 0, 0).ok()?,
    })
}

/// A function formatting a date and time with one ICU field set.
macro_rules! formatted {
    ($name:ident, $fieldset:ty, $make:expr) => {
        fn $name(date: jiff::civil::Date, time: jiff::civil::Time) -> String {
            // ICU's formatters aren't shared between threads: one each.
            thread_local! {
                static FORMATTER: Option<DateTimeFormatter<$fieldset>> =
                    DateTimeFormatter::try_new(prefs(), $make).ok();
            }
            FORMATTER.with(|formatter| match (formatter, datetime(date, time)) {
                (Some(formatter), Some(dt)) => formatter.format(&dt).to_string(),
                _ => date.to_string(),
            })
        }
    };
}

formatted!(long, fieldsets::MDE, fieldsets::MDE::long());
formatted!(short, fieldsets::MD, fieldsets::MD::medium());
formatted!(month, fieldsets::YM, fieldsets::YM::long());
formatted!(weekday, fieldsets::E, fieldsets::E::short());
formatted!(hour_minute, fieldsets::T, {
    let mut fs = fieldsets::T::medium();
    fs.time_precision = Some(TimePrecision::Minute);
    fs
});

/// "Thursday, October 29" or "október 29., csütörtök"
pub fn long_date(date: jiff::civil::Date) -> String {
    long(date, jiff::civil::Time::midnight())
}

/// "Oct 29" or "okt. 29."
pub fn short_date(date: jiff::civil::Date) -> String {
    short(date, jiff::civil::Time::midnight())
}

/// "October 2026" or "2026. október"
pub fn month_title(date: jiff::civil::Date) -> String {
    month(date, jiff::civil::Time::midnight())
}

/// "2:05 PM" or "14:05", as the region keeps time.
pub fn clock(time: jiff::civil::Time) -> String {
    hour_minute(jiff::civil::date(2026, 1, 1), time)
}

/// The weekday a locale's weeks start on, Monday as 0.
fn first_weekday_of(locale: &Locale) -> i64 {
    WeekInformation::try_new(locale.into()).map(|w| w.first_weekday as i64 - 1).unwrap_or(0)
}

fn first_weekday() -> i64 {
    static FIRST: LazyLock<i64> = LazyLock::new(|| first_weekday_of(&LOCALE));
    *FIRST
}

/// Days before `date` in its week.
pub fn week_offset(date: jiff::civil::Date) -> i64 {
    (date.weekday().to_monday_zero_offset() as i64 - first_weekday()).rem_euclid(7)
}

/// The calendar's column heads, from the first day of the week.
pub fn weekday_heads() -> Vec<String> {
    // 2024-01-01 was a Monday.
    (0..7)
        .map(|i| {
            let day = jiff::civil::date(2024, 1, 1 + ((i + first_weekday()) % 7) as i8);
            weekday(day, jiff::civil::Time::midnight())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long_in(tag: &str) -> String {
        let locale = parse(tag).expect("a locale");
        let formatter = DateTimeFormatter::try_new((&locale).into(), fieldsets::MDE::long()).expect("data");
        let dt = datetime(jiff::civil::date(2026, 10, 29), jiff::civil::Time::midnight()).expect("a date");
        formatter.format(&dt).to_string()
    }

    #[test]
    fn locales() {
        assert_eq!(parse("hu_HU.UTF-8").map(|l| l.to_string()), Some("hu-HU".into()));
        assert_eq!(parse("sr_RS@latin").map(|l| l.to_string()), Some("sr-RS".into()));
        assert!(parse("C").is_none() && parse("POSIX").is_none() && parse("").is_none());
    }

    #[test]
    fn dates_in_their_region() {
        assert_eq!(long_in("en_US.UTF-8"), "Thursday, October 29");
        assert!(long_in("hu_HU.UTF-8").starts_with("október 29."), "{}", long_in("hu_HU.UTF-8"));
        assert!(long_in("de_DE.UTF-8").contains("29. Oktober"), "{}", long_in("de_DE.UTF-8"));
        assert!(long_in("ja_JP.UTF-8").contains("10月29日"), "{}", long_in("ja_JP.UTF-8"));
    }

    #[test]
    fn weeks_start_where_the_region_says() {
        assert_eq!(first_weekday_of(&parse("en_US").unwrap()), 6, "Sunday");
        assert_eq!(first_weekday_of(&parse("hu_HU").unwrap()), 0, "Monday");
        assert_eq!(first_weekday_of(&parse("en_GB").unwrap()), 0, "Monday");
    }

    /// The ids in a Fluent file.
    fn ids(ftl: &str) -> Vec<String> {
        ftl.lines()
            .filter_map(|l| l.split_once(" = ").map(|(id, _)| id.trim().to_string()))
            .filter(|id| !id.starts_with('#') && !id.is_empty())
            .collect()
    }

    #[test]
    fn every_translation_is_whole_and_parses() {
        let english: Vec<String> =
            KEYS.iter().chain(WITH_APPS.iter()).map(|k| k.replace('_', "-")).collect();
        let mut languages = 0;
        for file in Localizations::iter() {
            let text = Localizations::get(&file).expect("embedded");
            let text = std::str::from_utf8(&text.data).expect("UTF-8");
            let found = ids(text);
            for id in &english {
                assert!(found.contains(id), "{file} lacks {id}");
            }
            for id in &found {
                assert!(english.contains(id), "{file} has unknown {id}");
            }
            for id in WITH_APPS {
                let line = text.lines().find(|l| l.starts_with(&format!("{id} ="))).expect("there");
                assert!(line.contains("{ $apps }"), "{file}: {id} lacks the app names");
            }
            let lang: unic_langid::LanguageIdentifier = file.split('/').next().unwrap().parse().expect("a language");
            i18n_embed::select(&*LOADER, &Localizations, &[lang.clone()]).expect("loads");
            assert!(LOADER.has("wifi"), "{file} does not load");
            languages += 1;
        }
        assert!(languages >= 2);
    }
}
