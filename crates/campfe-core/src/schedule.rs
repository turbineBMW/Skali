//! A schedule as a list of days: what the upcoming report holds for one
//! schedule, each entry under the day it starts (or the first day shown,
//! for one already under way), all-day entries first, then by time, then
//! what's due.

use crate::models::{DueItem, ScheduleEntry, UpcomingSchedule, When};
use chrono::{NaiveDate, TimeZone};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgendaItem<'a> {
    Entry(&'a ScheduleEntry),
    Due(&'a DueItem),
}

/// The days from `from` on with something on them, in order. Entries are
/// those of the schedule `schedule_id`; what's due is that of the project
/// `due_in_project`, if given (the schedule shows what's due).
pub fn agenda<'a, Tz: TimeZone>(
    upcoming: &'a UpcomingSchedule,
    schedule_id: i64,
    due_in_project: Option<i64>,
    from: NaiveDate,
    tz: &Tz,
) -> Vec<(NaiveDate, Vec<AgendaItem<'a>>)> {
    let mut days: BTreeMap<NaiveDate, Vec<AgendaItem<'a>>> = BTreeMap::new();
    let entries = upcoming
        .schedule_entries
        .iter()
        .chain(&upcoming.recurring_schedule_entry_occurrences)
        .filter(|entry| entry.recording.parent.as_ref().map(|p| p.id) == Some(schedule_id));
    for entry in entries {
        let Some(starts_at) = entry.starts_at else {
            continue;
        };
        let day = starts_at.date_in(tz).max(from);
        days.entry(day).or_default().push(AgendaItem::Entry(entry));
    }
    if let Some(project_id) = due_in_project {
        for item in &upcoming.assignables {
            if item.bucket.id != project_id || item.completed {
                continue;
            }
            if let Some(due) = item.due_on {
                days.entry(due.max(from))
                    .or_default()
                    .push(AgendaItem::Due(item));
            }
        }
    }
    for items in days.values_mut() {
        items.sort_by_key(|item| match item {
            AgendaItem::Entry(entry) => match entry.starts_at {
                Some(When::At(at)) if !entry.all_day => (1, Some(at)),
                _ => (0, None),
            },
            AgendaItem::Due(_) => (2, None),
        });
    }
    days.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Bucket, Parent, Recording};

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn entry(summary: &str, schedule: i64, all_day: bool, starts_at: When) -> ScheduleEntry {
        ScheduleEntry {
            recording: Recording {
                parent: Some(Parent {
                    id: schedule,
                    ..Default::default()
                }),
                ..Default::default()
            },
            summary: summary.into(),
            all_day,
            starts_at: Some(starts_at),
            ..Default::default()
        }
    }

    fn summaries(items: &[AgendaItem]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                AgendaItem::Entry(entry) => entry.summary.clone(),
                AgendaItem::Due(due) => format!("due: {}", due.content),
            })
            .collect()
    }

    #[test]
    fn days_in_order_all_day_first_then_by_time_then_due() {
        let at = |s: &str| When::At(s.parse().unwrap());
        let upcoming = UpcomingSchedule {
            schedule_entries: vec![
                entry("Afternoon", 7, false, at("2026-10-13T19:00:00Z")),
                entry("Release", 7, true, When::Day(day("2026-10-13"))),
                entry("Elsewhere", 8, false, at("2026-10-13T15:00:00Z")),
                // Began before the window: shown on its first day.
                entry("Conference", 7, true, When::Day(day("2026-10-08"))),
            ],
            recurring_schedule_entry_occurrences: vec![entry(
                "Morning sync",
                7,
                false,
                at("2026-10-13T14:00:00Z"),
            )],
            assignables: vec![
                DueItem {
                    content: "Ship it".into(),
                    due_on: Some(day("2026-10-13")),
                    bucket: Bucket {
                        id: 1,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                DueItem {
                    content: "Other project".into(),
                    due_on: Some(day("2026-10-13")),
                    bucket: Bucket {
                        id: 2,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ],
        };
        let new_york = chrono::FixedOffset::west_opt(4 * 3600).unwrap();
        let days = agenda(&upcoming, 7, Some(1), day("2026-10-10"), &new_york);
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].0, day("2026-10-10"));
        assert_eq!(summaries(&days[0].1), ["Conference"]);
        assert_eq!(days[1].0, day("2026-10-13"));
        assert_eq!(
            summaries(&days[1].1),
            ["Release", "Morning sync", "Afternoon", "due: Ship it"]
        );

        // Without what's due; and 19:00 UTC is the next day in Tokyo.
        let tokyo = chrono::FixedOffset::east_opt(9 * 3600).unwrap();
        let days = agenda(&upcoming, 7, None, day("2026-10-10"), &tokyo);
        let dates: Vec<_> = days.iter().map(|(d, _)| *d).collect();
        assert_eq!(
            dates,
            [day("2026-10-10"), day("2026-10-13"), day("2026-10-14")]
        );
        assert_eq!(summaries(&days[2].1), ["Afternoon"]);
    }
}
