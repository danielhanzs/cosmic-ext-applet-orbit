// SPDX-License-Identifier: GPL-3.0-only

//! Events from the calendars synced to the system: Evolution Data Server,
//! which GNOME Online Accounts (Google, Microsoft, Nextcloud…) fills.

use std::collections::HashMap;

use jiff::{Zoned, civil, tz::TimeZone};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SOURCES: &str = "org.gnome.evolution.dataserver.Sources5";
const FACTORY: &str = "org.gnome.evolution.dataserver.Calendar8";

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub summary: String,
    /// None: all day.
    pub start: Option<civil::Time>,
    pub end: Option<civil::Time>,
    pub location: String,
    /// A video meeting to join, found in the event's text.
    pub link: Option<String>,
}

/// Each day's events of the month `first` starts; None: no calendar service
/// on the system.
pub async fn month(first: civil::Date) -> Option<HashMap<civil::Date, Vec<Event>>> {
    match read(first.first_of_month()).await {
        Ok(days) => Some(days),
        Err(err) => {
            tracing::info!("calendars: {err}");
            None
        }
    }
}

async fn read(first: civil::Date) -> zbus::Result<HashMap<civil::Date, Vec<Event>>> {
    let conn = crate::bus::session().await?;
    let objects = zbus::fdo::ObjectManagerProxy::builder(&conn)
        .destination(SOURCES)?
        .path("/org/gnome/evolution/dataserver/SourceManager")?
        .build()
        .await?
        .get_managed_objects()
        .await?;
    let mut calendars = Vec::new();
    for ifaces in objects.values() {
        let Some((_, props)) =
            ifaces.iter().find(|(name, _)| name.as_str() == "org.gnome.evolution.dataserver.Source")
        else {
            continue;
        };
        let props: &HashMap<String, OwnedValue> = props;
        let string = |key: &str| props.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok());
        let (Some(uid), Some(data)) = (string("UID"), string("Data")) else { continue };
        if !is_shown_calendar(&data) {
            continue;
        }
        match open(&conn, &uid).await {
            Ok(calendar) => calendars.push(calendar),
            Err(err) => tracing::warn!("calendar {uid}: {err}"),
        }
    }

    let tz = TimeZone::system();
    let utc = |time: civil::DateTime| -> String {
        time.to_zoned(tz.clone())
            .map(|z| z.timestamp().strftime("%Y%m%dT%H%M%SZ").to_string())
            .unwrap_or_default()
    };
    // One query a day: EDS works out which repeating events fall on it.
    // The days are asked all at once.
    let day_events = |offset: i8| {
        let (conn, calendars, tz, utc) = (&conn, &calendars, &tz, &utc);
        async move {
            let day = first.saturating_add(jiff::Span::new().days(offset));
            let start = day.to_datetime(civil::Time::midnight());
            let query = format!(
                "(occur-in-time-range? (make-time \"{}\") (make-time \"{}\"))",
                utc(start),
                utc(start.saturating_add(jiff::Span::new().days(1))),
            );
            let mut events = Vec::new();
            for (bus, path) in calendars {
                let reply = conn
                    .call_method(
                        Some(bus.as_str()),
                        path,
                        Some("org.gnome.evolution.dataserver.Calendar"),
                        "GetObjectList",
                        &(query.as_str(),),
                    )
                    .await;
                match reply.and_then(|r| r.body().deserialize::<Vec<String>>().map_err(Into::into)) {
                    Ok(objects) => events.extend(objects.iter().flat_map(|ics| parse(ics, tz))),
                    Err(err) => tracing::warn!("calendar query: {err}"),
                }
            }
            events.sort_by_key(|e| e.start);
            events.dedup();
            (day, events)
        }
    };
    let days: HashMap<_, _> = futures::future::join_all((0..first.days_in_month()).map(day_events))
        .await
        .into_iter()
        .filter(|(_, events)| !events.is_empty())
        .collect();
    Ok(days)
}

/// A source that is a calendar, enabled and selected to show.
fn is_shown_calendar(keyfile: &str) -> bool {
    let mut section = "";
    let (mut calendar, mut enabled, mut selected) = (false, true, true);
    for line in keyfile.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name;
            calendar |= name == "Calendar";
        } else if let Some((key, value)) = line.split_once('=') {
            match (section, key.trim()) {
                ("Data Source", "Enabled") => enabled = value.trim() == "true",
                ("Calendar", "Selected") => selected = value.trim() == "true",
                _ => {}
            }
        }
    }
    calendar && enabled && selected
}

/// Opens a calendar: the bus and path to ask it at.
async fn open(conn: &zbus::Connection, uid: &str) -> zbus::Result<(String, OwnedObjectPath)> {
    let reply = conn
        .call_method(
            Some(FACTORY),
            "/org/gnome/evolution/dataserver/CalendarFactory",
            Some("org.gnome.evolution.dataserver.CalendarFactory"),
            "OpenCalendar",
            &(uid,),
        )
        .await?;
    let (path, bus): (OwnedObjectPath, String) = reply.body().deserialize()?;
    conn.call_method(Some(bus.as_str()), &path, Some("org.gnome.evolution.dataserver.Calendar"), "Open", &())
        .await?;
    Ok((bus, path))
}

/// The events of an iCalendar text. EDS already matched them to the day, so
/// a repeating event's start counts only for its time of day.
fn parse(ics: &str, tz: &TimeZone) -> Vec<Event> {
    // Long lines go on in lines that start with a space.
    let unfolded = ics.replace("\r\n ", "").replace("\n ", "").replace('\r', "");
    let mut events = Vec::new();
    let mut current: Option<Event> = None;
    // Components inside the event (alarms…) have SUMMARYs of their own.
    let mut depth = 0;
    let mut texts = String::new();
    for line in unfolded.lines() {
        match line {
            "BEGIN:VEVENT" => {
                current = Some(Event {
                    summary: String::new(),
                    start: None,
                    end: None,
                    location: String::new(),
                    link: None,
                });
                depth = 0;
                texts.clear();
            }
            "END:VEVENT" => {
                if let Some(mut event) = current.take() {
                    event.link = meeting_link(&texts);
                    events.push(event);
                }
            }
            _ if current.is_some() && line.starts_with("BEGIN:") => depth += 1,
            _ if current.is_some() && line.starts_with("END:") => depth -= 1,
            _ if depth > 0 => {}
            _ => {
                let Some(event) = current.as_mut() else { continue };
                let Some((name, value)) = line.split_once(':') else { continue };
                let (key, params) = name.split_once(';').unwrap_or((name, ""));
                let value = value.replace("\\,", ",").replace("\\;", ";").replace("\\n", " ");
                if matches!(key, "LOCATION" | "DESCRIPTION" | "URL" | "X-GOOGLE-CONFERENCE") {
                    texts.push_str(&value);
                    texts.push(' ');
                }
                match key {
                    "SUMMARY" => event.summary = value,
                    "LOCATION" => event.location = value,
                    "DTSTART" => event.start = time(&value, params, tz),
                    "DTEND" => event.end = time(&value, params, tz),
                    _ => {}
                }
            }
        }
    }
    events
}

/// The first video-meeting address in a text: HTTPS, on a meeting
/// service's own domain or a subdomain of it.
fn meeting_link(text: &str) -> Option<String> {
    const SERVICES: [&str; 8] = [
        "meet.google.com",
        "zoom.us",
        "teams.microsoft.com",
        "teams.live.com",
        "meet.jit.si",
        "whereby.com",
        "webex.com",
        "gotomeeting.com",
    ];
    text.match_indices("https://")
        .map(|(at, _)| {
            let rest = &text[at..];
            let end = rest.find(|c: char| c.is_whitespace() || "<>\"'(),;".contains(c)).unwrap_or(rest.len());
            &rest[..end]
        })
        // Read as browsers read URLs, so the host checked is the one opened.
        .filter_map(|text| url::Url::parse(text).ok())
        .find(|url| {
            let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && SERVICES.iter().any(|s| host == *s || host.ends_with(&format!(".{s}")))
        })
        .map(String::from)
}

/// The local time of day of an iCalendar date-time; None for a whole day.
fn time(value: &str, params: &str, tz: &TimeZone) -> Option<civil::Time> {
    if params.contains("VALUE=DATE") && !params.contains("VALUE=DATE-TIME") || value.len() == 8 {
        return None;
    }
    let (text, utc) = match value.strip_suffix('Z') {
        Some(text) => (text, true),
        None => (value, false),
    };
    let local = civil::DateTime::strptime("%Y%m%dT%H%M%S", text).ok()?;
    if utc {
        let zoned: Zoned = local.to_zoned(TimeZone::UTC).ok()?.with_time_zone(tz.clone());
        return Some(zoned.time());
    }
    // ponytail: a TZID other than ours is read as local time; convert with jiff's tzdb if that matters.
    Some(local.time())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_events() {
        let ics = "BEGIN:VEVENT\r\nSUMMARY:Sprint review\\, team\r\nDTSTART;TZID=Europe/Budapest:20261029T140000\r\nDTEND;TZID=Europe/Budapest:20261029T15\r\n 0000\r\nLOCATION:Meet\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261029\r\nEND:VEVENT\r\n";
        let events = parse(ics, &TimeZone::UTC);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].summary, "Sprint review, team");
        assert_eq!(events[0].start, Some(civil::time(14, 0, 0, 0)));
        assert_eq!(events[0].end, Some(civil::time(15, 0, 0, 0)));
        assert_eq!(events[0].location, "Meet");
        assert_eq!(events[1].start, None);
    }

    #[test]
    fn alarms_keep_their_own_summary() {
        let ics = "BEGIN:VEVENT\r\nSUMMARY:Standup\r\nDESCRIPTION:Join: https://meet.google.com/abc-defg-hij\\nor call\r\nDTSTART:20261029T090000Z\r\nBEGIN:VALARM\r\nACTION:EMAIL\r\nSUMMARY:Alarm notification\r\nEND:VALARM\r\nEND:VEVENT\r\n";
        let events = parse(ics, &TimeZone::UTC);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, "Standup");
        assert_eq!(events[0].link.as_deref(), Some("https://meet.google.com/abc-defg-hij"));
    }

    #[test]
    fn meeting_links() {
        assert_eq!(meeting_link("Room 4, https://example.com/x"), None);
        assert_eq!(meeting_link("https://evil.com/?x=meet.google.com/abc"), None);
        assert_eq!(meeting_link("https://meet.google.com@evil.com/abc"), None);
        assert_eq!(meeting_link("https://notzoom.us/j/1"), None);
        assert_eq!(meeting_link("http://zoom.us/j/1"), None);
        // Read as a browser reads it: the host is zoom.us, "@evil.com" is path.
        assert_eq!(meeting_link("https://zoom.us\\@evil.com/j/1"), Some("https://zoom.us/@evil.com/j/1".into()));
        assert_eq!(meeting_link("https://evil.com%2Fzoom.us/j/1"), None);
        assert_eq!(
            meeting_link("<https://us02web.zoom.us/j/123?pwd=x>"),
            Some("https://us02web.zoom.us/j/123?pwd=x".into())
        );
    }

    #[test]
    fn utc_to_local() {
        let tz = TimeZone::fixed(jiff::tz::offset(1));
        assert_eq!(time("20261029T130000Z", "", &tz), Some(civil::time(14, 0, 0, 0)));
    }

    #[test]
    fn shown_calendars() {
        assert!(is_shown_calendar("[Data Source]\nDisplayName=Work\nEnabled=true\n\n[Calendar]\nBackendName=caldav\nSelected=true\n"));
        assert!(!is_shown_calendar("[Data Source]\nEnabled=true\n[Address Book]\n"));
        assert!(!is_shown_calendar("[Data Source]\nEnabled=false\n[Calendar]\n"));
    }
}
