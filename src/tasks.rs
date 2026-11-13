// SPDX-License-Identifier: GPL-3.0-only

//! The to-dos of the task lists synced to the system (Evolution Data
//! Server: Google Tasks, Nextcloud, local lists…), the open ones, and
//! ticking them off.

use jiff::civil;
use zbus::zvariant::OwnedObjectPath;

#[derive(Debug, Clone)]
pub struct Task {
    /// Its own id, which stays as lists are read again.
    pub uid: String,
    pub summary: String,
    pub due: Option<civil::Date>,
    /// Its list, and itself as EDS gave it, to write back done.
    list: (String, OwnedObjectPath),
    ics: String,
}

/// The open to-dos, the soonest due first; None: no task service.
pub async fn open() -> Option<Vec<Task>> {
    read().await.inspect_err(|err| tracing::info!("tasks: {err}")).ok()
}

async fn read() -> zbus::Result<Vec<Task>> {
    let conn = crate::bus::session().await?;
    let mut tasks = Vec::new();
    for list in crate::calendar::open_all(&conn, "Task List", "OpenTaskList").await? {
        let reply = conn
            .call_method(Some(list.0.as_str()), &list.1, Some(CALENDAR), "GetObjectList", &("#t",))
            .await
            .and_then(|r| r.body().deserialize::<Vec<String>>().map_err(Into::into));
        match reply {
            Ok(objects) => tasks.extend(objects.into_iter().filter_map(|ics| {
                let open = parse(&ics)?;
                Some(Task { uid: open.uid, summary: open.summary, due: open.due, list: list.clone(), ics })
            })),
            Err(err) => tracing::warn!("task list: {err}"),
        }
    }
    // Undated ones last.
    tasks.sort_by(|a, b| (a.due.is_none(), a.due, &a.summary).cmp(&(b.due.is_none(), b.due, &b.summary)));
    Ok(tasks)
}

const CALENDAR: &str = "org.gnome.evolution.dataserver.Calendar";

/// Ticks a to-do off in its list.
pub async fn complete(task: Task) {
    let now = jiff::Timestamp::now().strftime("%Y%m%dT%H%M%SZ").to_string();
    let ics = done(&task.ics, &now);
    let conn = match crate::bus::session().await {
        Ok(conn) => conn,
        Err(err) => return tracing::warn!("task: {err}"),
    };
    let reply = conn
        .call_method(Some(task.list.0.as_str()), &task.list.1, Some(CALENDAR), "ModifyObjects", &(vec![ics], "this", 0u32))
        .await;
    if let Err(err) = reply {
        tracing::warn!("task done: {err}");
    }
}

/// Long lines go on in lines that start with a space or a tab.
fn unfold(ics: &str) -> String {
    ics.replace("\r\n ", "").replace("\r\n\t", "").replace("\n ", "").replace("\n\t", "").replace('\r', "")
}

struct Open {
    uid: String,
    summary: String,
    due: Option<civil::Date>,
}

/// An open to-do; None: done, or not a to-do. Only the VTODO's own lines
/// count, not those of what's inside it (alarms).
fn parse(ics: &str) -> Option<Open> {
    let (mut uid, mut summary, mut due, mut done, mut todo) = (String::new(), None, None, false, false);
    let mut inner = 0;
    for line in unfold(ics).lines() {
        match line {
            "BEGIN:VTODO" => todo = true,
            "END:VTODO" => break,
            _ if !todo => {}
            _ if line.starts_with("BEGIN:") => inner += 1,
            _ if line.starts_with("END:") => inner -= 1,
            _ if inner > 0 => {}
            _ => {
                let Some((name, value)) = line.split_once(':') else { continue };
                match name.split(';').next().unwrap_or(name) {
                    "UID" => uid = value.to_string(),
                    "SUMMARY" => summary = Some(value.replace("\\,", ",").replace("\\;", ";").replace("\\n", " ")),
                    "STATUS" => done |= value == "COMPLETED" || value == "CANCELLED",
                    "COMPLETED" => done = true,
                    "DUE" => due = due_day(value),
                    _ => {}
                }
            }
        }
    }
    (todo && !done).then(|| Open { uid, summary: summary.unwrap_or_default(), due })
}

/// "20261110", or "20261110T120000" (floating), or "…Z" (UTC, so the day
/// here may differ).
fn due_day(value: &str) -> Option<civil::Date> {
    if let Some(utc) = value.strip_suffix('Z') {
        let at = civil::DateTime::strptime("%Y%m%dT%H%M%S", utc).ok()?;
        return Some(at.to_zoned(jiff::tz::TimeZone::UTC).ok()?.with_time_zone(jiff::tz::TimeZone::system()).date());
    }
    civil::Date::strptime("%Y%m%d", value.get(..8)?).ok()
}

/// The to-do marked done at `now` (UTC, "20261110T120000Z"): its own
/// status lines replaced, wherever the VTODO sits.
fn done(ics: &str, now: &str) -> String {
    let mut out = Vec::new();
    let (mut todo, mut inner) = (false, 0);
    for line in unfold(ics).lines() {
        let own = todo && inner == 0;
        match line {
            "BEGIN:VTODO" => todo = true,
            "END:VTODO" => {
                out.extend(["STATUS:COMPLETED".to_string(), format!("COMPLETED:{now}"), "PERCENT-COMPLETE:100".to_string()]);
                todo = false;
            }
            _ if todo && line.starts_with("BEGIN:") => inner += 1,
            _ if todo && line.starts_with("END:") => inner -= 1,
            _ => {}
        }
        let name = line.split([':', ';']).next().unwrap_or("");
        if !(own && matches!(name, "STATUS" | "COMPLETED" | "PERCENT-COMPLETE")) {
            out.push(line.to_string());
        }
    }
    out.join("\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODO: &str = "BEGIN:VTODO\r\nUID:1\r\nSUMMARY:Buy milk\\, \r\n eggs\r\nDUE;VALUE=DATE:20261112\r\n\
                        STATUS:NEEDS-ACTION\r\nBEGIN:VALARM\r\nSTATUS:X\r\nEND:VALARM\r\nEND:VTODO";

    #[test]
    fn reads_and_ticks_off() {
        let open = parse(TODO).unwrap();
        assert_eq!((open.uid.as_str(), open.summary.as_str(), open.due), ("1", "Buy milk, eggs", Some(civil::date(2026, 11, 12))));
        let ticked = done(TODO, "20261110T080000Z");
        assert!(ticked.contains("STATUS:COMPLETED\r\nCOMPLETED:20261110T080000Z\r\nPERCENT-COMPLETE:100\r\nEND:VTODO"));
        assert!(!ticked.contains("NEEDS-ACTION"));
        // The alarm's own lines stay.
        assert!(ticked.contains("BEGIN:VALARM\r\nSTATUS:X\r\nEND:VALARM"));
        assert!(parse(&ticked).is_none());
        // Inside a calendar too.
        let wrapped = format!("BEGIN:VCALENDAR\r\n{TODO}\r\nEND:VCALENDAR");
        assert!(parse(&wrapped).is_some());
        assert!(parse(&done(&wrapped, "20261110T080000Z")).is_none());
        assert!(parse("BEGIN:VEVENT\r\nSUMMARY:x\r\nEND:VEVENT").is_none());
    }
}
