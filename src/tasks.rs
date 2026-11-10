// SPDX-License-Identifier: GPL-3.0-only

//! The to-dos of the task lists synced to the system (Evolution Data
//! Server: Google Tasks, Nextcloud, local lists…), the open ones, and
//! ticking them off.

use jiff::civil;
use zbus::zvariant::OwnedObjectPath;

#[derive(Debug, Clone)]
pub struct Task {
    pub summary: String,
    pub due: Option<civil::Date>,
    /// Its list, and itself as EDS gave it, to write back done.
    list: (String, OwnedObjectPath),
    ics: String,
}

/// The open to-dos, the soonest due first; None: no task service.
pub async fn open() -> Option<Vec<Task>> {
    match read().await {
        Ok(tasks) => Some(tasks),
        Err(err) => {
            tracing::info!("tasks: {err}");
            None
        }
    }
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
                let (summary, due) = parse(&ics)?;
                Some(Task { summary, due, list: list.clone(), ics })
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

fn unfold(ics: &str) -> String {
    ics.replace("\r\n ", "").replace("\n ", "").replace('\r', "")
}

/// An open to-do's summary and due date; None: done, or not a to-do.
fn parse(ics: &str) -> Option<(String, Option<civil::Date>)> {
    let (mut summary, mut due, mut done, mut todo) = (None, None, false, false);
    let mut depth = 0;
    for line in unfold(ics).lines() {
        if line == "BEGIN:VTODO" {
            todo = true;
            continue;
        }
        if line.starts_with("BEGIN:") && todo {
            depth += 1;
        } else if line.starts_with("END:") && depth > 0 {
            depth -= 1;
        } else if depth == 0 && todo {
            let Some((name, value)) = line.split_once(':') else { continue };
            match name.split(';').next().unwrap_or(name) {
                "SUMMARY" => summary = Some(value.replace("\\,", ",").replace("\\;", ";").replace("\\n", " ")),
                "STATUS" => done |= value == "COMPLETED" || value == "CANCELLED",
                "COMPLETED" => done = true,
                // "20261110" or "20261110T120000Z": the day is enough.
                "DUE" => due = value.get(..8).and_then(|d| civil::Date::strptime("%Y%m%d", d).ok()),
                _ => {}
            }
        }
    }
    (todo && !done).then(|| (summary.unwrap_or_default(), due))
}

/// The to-do marked done at `now` (UTC, "20261110T120000Z").
fn done(ics: &str, now: &str) -> String {
    let mut out = Vec::new();
    let mut depth = 0;
    for line in unfold(ics).lines() {
        if line.starts_with("BEGIN:") {
            depth += 1;
        }
        let name = line.split([':', ';']).next().unwrap_or("");
        let own = depth == 1 && matches!(name, "STATUS" | "COMPLETED" | "PERCENT-COMPLETE");
        if depth == 1 && line == "END:VTODO" {
            out.extend(["STATUS:COMPLETED".to_string(), format!("COMPLETED:{now}"), "PERCENT-COMPLETE:100".to_string()]);
        }
        if line.starts_with("END:") {
            depth -= 1;
        }
        if !own {
            out.push(line.to_string());
        }
    }
    out.join("\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODO: &str = "BEGIN:VTODO\r\nUID:1\r\nSUMMARY:Buy milk\\, eggs\r\nDUE;VALUE=DATE:20261112\r\n\
                        STATUS:NEEDS-ACTION\r\nBEGIN:VALARM\r\nSTATUS:X\r\nEND:VALARM\r\nEND:VTODO";

    #[test]
    fn reads_and_ticks_off() {
        assert_eq!(parse(TODO), Some(("Buy milk, eggs".to_string(), Some(civil::date(2026, 11, 12)))));
        let ticked = done(TODO, "20261110T080000Z");
        assert!(ticked.contains("STATUS:COMPLETED\r\nCOMPLETED:20261110T080000Z\r\nPERCENT-COMPLETE:100\r\nEND:VTODO"));
        assert!(!ticked.contains("NEEDS-ACTION"));
        // The alarm's own lines stay.
        assert!(ticked.contains("BEGIN:VALARM\r\nSTATUS:X\r\nEND:VALARM"));
        assert_eq!(parse(&ticked), None);
        assert_eq!(parse("BEGIN:VEVENT\r\nSUMMARY:x\r\nEND:VEVENT"), None);
    }

    /// Lists this system's open to-dos: cargo test -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn lists_tasks() {
        let conn = crate::bus::session().await.unwrap();
        for list in crate::calendar::open_all(&conn, "Task List", "OpenTaskList").await.unwrap() {
            let all: Vec<String> = conn
                .call_method(Some(list.0.as_str()), &list.1, Some(CALENDAR), "GetObjectList", &("#t",))
                .await
                .unwrap()
                .body()
                .deserialize()
                .unwrap();
            println!("{} objects in {}", all.len(), list.1);
        }
        for task in open().await.expect("a task service") {
            println!("{:?} {}", task.due, task.summary);
        }
    }
}
